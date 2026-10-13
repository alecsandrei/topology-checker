"""Manage preloaded Rust and native QGIS benchmark sessions."""

import json
import os
import select
import subprocess
import time
from pathlib import Path
from typing import Any

from .fixtures import HERE, MAPPINGS, ROOT


class EngineFailure(RuntimeError):
    """An engine failed, timed out or returned an invalid response."""

    def __init__(self, status: str, message: str) -> None:
        """Retain a machine-readable failure category."""
        super().__init__(message)
        self.status = status


def atomic_json(path: Path, value: Any) -> None:
    """Write JSON atomically so an interrupted run retains previous results."""
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")
    temporary.replace(path)


def wait_file(
    path: Path,
    process: subprocess.Popen,
    timeout: float,
    validation_timeout: float | None = None,
) -> dict:
    """Wait for an atomic response, observing process exit and a deadline."""
    deadline = time.monotonic() + timeout
    current_phase = "preparing"
    while not path.exists():
        marker = path.parent / "phase.txt"
        if validation_timeout is not None and marker.exists():
            new_phase = marker.read_text()
            if new_phase != current_phase:
                current_phase = new_phase
                allowance = validation_timeout if new_phase == "validating" else timeout
                deadline = time.monotonic() + allowance
        if process.poll() is not None:
            raise EngineFailure("failed", f"Engine exited: {process.returncode}")
        if time.monotonic() >= deadline:
            raise EngineFailure("timeout", f"Deadline exceeded during {current_phase}")
        time.sleep(0.02)
    response = json.loads(path.read_text())
    path.unlink()
    if response["status"] == "failed":
        raise EngineFailure("failed", response["error"])
    return response


def close_process(process: subprocess.Popen) -> None:
    """Stop only a benchmark-owned process and reap it."""
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
    for stream in (process.stdin, process.stdout, process.stderr):
        if stream is not None:
            stream.close()


class QgisSession:
    """One isolated QGIS profile, with one rule configured per sample."""

    def __init__(
        self, executable: str, directory: Path, startup_timeout: float
    ) -> None:
        """Start the actual GUI binary, leaving regular profiles untouched."""
        self.directory = directory
        self.startup_timeout = startup_timeout
        directory.mkdir(parents=True, exist_ok=True)
        profile = directory / "profiles" / "profiles" / "benchmark" / "qgis.org"
        profile.mkdir(parents=True, exist_ok=True)
        settings = "[Plugins]\nlibplugin_topology=true\n[authentication]\nuse-password-helper=false\ngenerate-random-password-for-keychain=false\n[locale]\noverrideFlag=true\nuserLocale=en_US\n"
        (profile / "QGIS4.ini").write_text(settings)
        (profile / "QGIS3.ini").write_text(settings)
        global_settings = directory / "settings.ini"
        global_settings.write_text(settings)
        self.log = (directory / "qgis.log").open("w")
        environment = dict(
            os.environ,
            TOPOLOGY_QGIS_SESSION=str(directory.resolve()),
            QGIS_CONTINUOUS_INTEGRATION_RUN="true",
            QT_QPA_PLATFORM=os.environ.get("QT_QPA_PLATFORM", "offscreen"),
        )
        # QGIS embeds system Python. A runner's venv can otherwise redirect its
        # interpreter prefix and hide the distribution's Qt bindings.
        virtual_env = environment.pop("VIRTUAL_ENV", None)
        environment.pop("PYTHONHOME", None)
        if virtual_env:
            environment["PATH"] = os.pathsep.join(
                part
                for part in environment.get("PATH", "").split(os.pathsep)
                if Path(part) != Path(virtual_env) / "bin"
            )
        # All IPC files are unique to this session, never to the user's app.
        self.process = subprocess.Popen(
            [
                executable,
                "--nologo",
                "--noversioncheck",
                "--hide-browser",
                "--profiles-path",
                str((directory / "profiles").resolve()),
                "--profile",
                "benchmark",
                "--globalsettingsfile",
                str(global_settings.resolve()),
                "--code",
                str(HERE / "qgis_startup.py"),
            ],
            env=environment,
            stdout=self.log,
            stderr=self.log,
        )
        try:
            self.metadata = wait_file(
                directory / "response.json", self.process, startup_timeout
            )
        except (EngineFailure, ValueError, OSError):
            self.close()
            raise

    def command(self, command: dict, timeout: float) -> dict:
        """Submit one command and wait until its validation has completed."""
        atomic_json(self.directory / "request.json", command)
        return wait_file(self.directory / "response.json", self.process, timeout)

    def load(self, fixture: dict, timeout: float) -> None:
        """Preload geometries before any sample requests."""
        self.command(
            {
                "action": "load",
                "input": str(ROOT / fixture["input"]["path"]),
                "other": (
                    str(ROOT / fixture["other"]["path"]) if "other" in fixture else None
                ),
            },
            timeout,
        )

    def sample(self, rule: str, timeout: float) -> dict:
        """Time the plugin's Validate All slot, including rows and markers."""
        (self.directory / "phase.txt").unlink(missing_ok=True)
        atomic_json(
            self.directory / "request.json",
            {"action": "sample", "rule": MAPPINGS[rule][0]},
        )
        return wait_file(
            self.directory / "response.json",
            self.process,
            self.startup_timeout,
            timeout,
        )

    def close(self) -> None:
        """Release the isolated app and its log handle."""
        close_process(self.process)
        self.log.close()


class RustSession:
    """A production importer followed by fresh layers for each request."""

    def __init__(
        self,
        executable: Path,
        rule: str,
        fixture: dict,
        directory: Path,
        startup_timeout: float,
    ) -> None:
        """Preload one fixture with Rayon constrained to a single thread."""
        self.log = (directory / "rust.log").open("w")
        self.startup_timeout = startup_timeout
        self.buffer = b""
        command = [str(executable), rule, str(ROOT / fixture["input"]["path"])]
        if "other" in fixture:
            command.append(str(ROOT / fixture["other"]["path"]))
        self.process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self.log,
            bufsize=0,
            env=dict(os.environ, RAYON_NUM_THREADS="1"),
        )
        try:
            self.metadata = self.read(startup_timeout)
        except (EngineFailure, ValueError, OSError):
            self.close()
            raise

    def read(self, timeout: float) -> dict:
        """Read exactly one newline response with an external deadline."""
        assert self.process.stdout is not None
        deadline = time.monotonic() + timeout
        while b"\n" not in self.buffer:
            remaining = max(0, deadline - time.monotonic())
            if not select.select([self.process.stdout], [], [], remaining)[0]:
                raise EngineFailure("timeout", f"No Rust response within {timeout:g}s")
            chunk = os.read(self.process.stdout.fileno(), 65536)
            if not chunk:
                raise EngineFailure("failed", f"Rust exited: {self.process.poll()}")
            self.buffer += chunk
        line, self.buffer = self.buffer.split(b"\n", 1)
        return json.loads(line)

    def sample(self, rule: str, timeout: float) -> dict:
        """Request one validation; IPC and serialization are outside its clock."""
        assert self.process.stdin is not None
        self.process.stdin.write(b"sample\n")
        self.process.stdin.flush()
        preparing = self.read(self.startup_timeout)
        if preparing["status"] != "validating":
            raise EngineFailure("failed", "Expected Rust validation phase")
        finished = self.read(timeout)
        if finished["status"] != "extracting":
            raise EngineFailure("failed", "Expected Rust extraction phase")
        return self.read(self.startup_timeout)

    def close(self) -> None:
        """Release the benchmark process and log handle."""
        close_process(self.process)
        self.log.close()
