"""Launch actual GIS engines in isolated benchmark sessions."""

import json
import os
import shutil
from pathlib import Path
from typing import Any

from benchmarks.qgis.fixtures import ROOT
from benchmarks.qgis.run import QgisSession, RustSession

from .catalog import GVSIG
from .common import WorkerSession
from .postgis import PostgisSession

HERE = Path(__file__).resolve().parent


class QgisAdapter(QgisSession):
    """Use the stock C++ plugin and convert its visible provenance."""

    def sample(self, rule: str, timeout: float) -> dict[str, Any]:
        """Keep primary IDs while converting the plugin's layer-name column."""
        response = super().sample(rule, timeout)
        for error in response.get("errors", []):
            error["layer"] = 0
        return response

    def close(self) -> None:
        """Retain logs and settings, removing this run's disposable GUI profile."""
        super().close()
        profiles = self.directory / "profiles"
        if profiles.is_dir():
            shutil.rmtree(profiles)


def create_session(
    engine: str,
    rule: str,
    fixture: dict[str, Any],
    directory: Path,
    startup_timeout: float,
) -> Any:
    """Start one workload, without including application startup in validation."""
    directory.mkdir(parents=True, exist_ok=True)
    if engine == "rust":
        return RustSession(
            Path(
                os.environ.get(
                    "GIS_RUST_BINARY",
                    str(ROOT / "target/release/examples/qgis_benchmark"),
                )
            ),
            rule,
            fixture,
            directory,
            startup_timeout,
        )
    if engine == "qgis":
        session = QgisAdapter(
            os.environ.get("GIS_QGIS_BINARY", "/usr/bin/qgis"),
            directory,
            startup_timeout,
        )
        try:
            if not str(session.metadata["version"]).startswith("4.2."):
                raise ValueError("The shared suite requires QGIS 4.2.x")
            session.load(fixture, startup_timeout)
        except (RuntimeError, OSError, ValueError):
            session.close()
            raise
        return session
    if engine == "postgis":
        return PostgisSession(fixture, directory, startup_timeout)
    environment = {
        "GIS_WORKER_DIRECTORY": str(directory.resolve()),
        "GIS_WORKER_FIXTURE": json.dumps(
            {name: str(ROOT / data["path"]) for name, data in fixture.items()}
        ),
        "GIS_WORKER_RULE": rule,
    }
    if engine == "grass":
        executable = os.environ.get(
            "GIS_GRASS_BINARY", shutil.which("grass") or "grass"
        )
        return WorkerSession(
            [
                executable,
                "--tmp-location",
                "EPSG:3857",
                "--exec",
                "/usr/bin/python3",
                str(HERE / "grass_worker.py"),
            ],
            directory,
            startup_timeout,
            environment,
        )
    if engine == "gvsig":
        environment["GIS_WORKER_MAPPINGS"] = json.dumps(GVSIG)
        return WorkerSession(
            ["bash", str(HERE / "launch_gvsig.sh"), str(directory.resolve())],
            directory,
            startup_timeout,
            environment,
        )
    if engine == "fme":
        command = os.environ.get("GIS_FME_WORKER_COMMAND")
        if not command:
            raise FileNotFoundError("No licensed FME IPC worker configured")
        parts = json.loads(command)
        if (
            not isinstance(parts, list)
            or not parts
            or not all(isinstance(p, str) for p in parts)
        ):
            raise ValueError("GIS_FME_WORKER_COMMAND must be a JSON argv array")
        return WorkerSession(parts, directory, startup_timeout, environment)
    raise ValueError(f"unknown engine: {engine}")
