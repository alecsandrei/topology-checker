"""Validate the four-platform release archives and their SHA-256 manifest."""

from __future__ import annotations

import argparse
import hashlib
import re
import sys
import tarfile
import zipfile
from collections.abc import Callable
from email.message import Message
from email.parser import BytesParser
from pathlib import Path
from typing import Any

from packaging.requirements import Requirement
from packaging.tags import parse_tag
from packaging.utils import (
    canonicalize_name,
    parse_sdist_filename,
    parse_wheel_filename,
)
from packaging.version import Version

if sys.version_info >= (3, 11):
    import tomllib
else:
    import tomli as tomllib

ROOT = Path(__file__).resolve().parents[1]
PLATFORMS = {"linux-x86_64", "windows-x86_64", "macos-x86_64", "macos-arm64"}


def require(condition: bool, message: str) -> None:
    """Raise a useful validation error when an invariant fails."""
    if not condition:
        raise ValueError(message)


def project_config(root: Path) -> tuple[dict[str, Any], str]:
    """Read Python metadata and the binding crate's single version source."""
    project = tomllib.loads((root / "pyproject.toml").read_text())["project"]
    crate = tomllib.loads((root / "crates/python/Cargo.toml").read_text())
    version = str(crate["package"]["version"])
    require(project.get("dynamic") == ["version"], "version must be dynamic")
    require(not Version(version).is_prerelease, "release must have a stable version")
    return project, version


def check_metadata(metadata: Message, project: dict[str, Any], version: str) -> None:
    """Check names, versions, dependencies, license declarations and Python support."""
    for field, expected in {
        "Name": project["name"],
        "Version": version,
        "Requires-Python": project["requires-python"],
        "License-Expression": project["license"],
    }.items():
        require(metadata.get_all(field) == [expected], f"incorrect metadata {field}")
    expected_requirements = [*project["dependencies"]]
    for extra, dependencies in project["optional-dependencies"].items():
        expected_requirements.extend(
            f'{dep}; extra == "{extra}"' for dep in dependencies
        )
    actual = {str(Requirement(dep)) for dep in metadata.get_all("Requires-Dist", [])}
    expected = {str(Requirement(dep)) for dep in expected_requirements}
    require(actual == expected, f"incorrect dependencies: {actual}")
    require(
        set(metadata.get_all("Provides-Extra", []))
        == set(project["optional-dependencies"]),
        "incorrect extras",
    )
    require(
        set(metadata.get_all("License-File", [])) == set(project["license-files"]),
        "incorrect or missing license file declarations",
    )
    require(
        set(project["classifiers"]) <= set(metadata.get_all("Classifier", [])),
        "missing classifiers",
    )


def check_files(
    read: Callable[[str], bytes], prefix: str, paths: list[str], root: Path
) -> None:
    """Require packaged text to match reviewed sources across checkout line endings."""
    for path in paths:
        require(
            read(prefix + path).replace(b"\r\n", b"\n")
            == (root / path).read_bytes().replace(b"\r\n", b"\n"),
            f"stale file: {path}",
        )


def platform_key(platform: str) -> str:
    """Accept only the four release platforms and the manylinux2014 baseline."""
    if platform in {"manylinux_2_17_x86_64", "manylinux2014_x86_64"}:
        return "linux-x86_64"
    if platform == "win_amd64":
        return "windows-x86_64"
    match = re.fullmatch(r"macosx_(\d+)_(\d+)_(x86_64|arm64)", platform)
    if match and int(match[1]) >= 10:
        if match[3] == "arm64":
            require(int(match[1]) >= 11, "invalid Apple Silicon deployment target")
        return f"macos-{match[3]}"
    raise ValueError(f"unsupported platform tag: {platform}")


def check_native(payload: bytes, platform: str) -> None:
    """Check that the extension binary format and machine match its wheel tag."""
    if platform == "linux-x86_64":
        valid = (
            payload[:6] == b"\x7fELF\x02\x01"
            and int.from_bytes(payload[18:20], "little") == 62
        )
    elif platform == "windows-x86_64":
        offset = int.from_bytes(payload[60:64], "little")
        valid = (
            payload[:2] == b"MZ"
            and payload[offset : offset + 4] == b"PE\0\0"
            and int.from_bytes(payload[offset + 4 : offset + 6], "little") == 0x8664
        )
    else:
        machine = 0x0100000C if platform == "macos-arm64" else 0x01000007
        valid = (
            payload[:4] == b"\xcf\xfa\xed\xfe"
            and int.from_bytes(payload[4:8], "little") == machine
        )
    require(valid, f"native extension does not match {platform}")


def check_wheel(path: Path, project: dict[str, Any], version: str, root: Path) -> str:
    """Check ABI3 tags, metadata, license texts, native payload and typing files."""
    name, wheel_version, build, tags = parse_wheel_filename(path.name)
    require(
        name == canonicalize_name(project["name"]), f"wrong wheel name: {path.name}"
    )
    require(str(wheel_version) == version, f"mismatched version: {path.name}")
    require(not build, "wheel build tags are not part of this release")
    platforms = {platform_key(tag.platform) for tag in tags}
    require(len(platforms) == 1, "wheel spans more than one release platform")
    platform = platforms.pop()
    require(
        all(tag.interpreter == "cp310" and tag.abi == "abi3" for tag in tags),
        "expected cp310-abi3 wheel tags",
    )
    dist_info = f"topology_checker-{version}.dist-info/"
    with zipfile.ZipFile(path) as archive:
        require(archive.testzip() is None, f"corrupt wheel: {path.name}")
        metadata = BytesParser().parsebytes(archive.read(dist_info + "METADATA"))
        check_metadata(metadata, project, version)
        wheel = BytesParser().parsebytes(archive.read(dist_info + "WHEEL"))
        require(wheel.get("Root-Is-Purelib") == "false", "wheel must be native")
        internal_tags = {
            item for tag in wheel.get_all("Tag", []) for item in parse_tag(tag)
        }
        require(frozenset(internal_tags) == tags, "filename and WHEEL tags disagree")
        check_files(
            archive.read, dist_info + "licenses/", project["license-files"], root
        )
        sources = [
            str(source.relative_to(root / "python")).replace("\\", "/")
            for source in (root / "python/topology_checker").iterdir()
            if source.suffix in {".py", ".pyi"} or source.name == "py.typed"
        ]
        for source in sources:
            require(
                archive.read(source).replace(b"\r\n", b"\n")
                == (root / "python" / source).read_bytes().replace(b"\r\n", b"\n"),
                f"missing or stale Python file: {source}",
            )
        suffix = ".pyd" if platform == "windows-x86_64" else ".so"
        extensions = [
            name
            for name in archive.namelist()
            if name.startswith("topology_checker/_native") and name.endswith(suffix)
        ]
        require(len(extensions) == 1, "expected exactly one native extension")
        check_native(archive.read(extensions[0]), platform)
    return platform


def check_sdist(path: Path, project: dict[str, Any], version: str, root: Path) -> None:
    """Check metadata, licensing and the source files needed for an isolated build."""
    name, sdist_version = parse_sdist_filename(path.name)
    require(name == canonicalize_name(project["name"]), "wrong source archive name")
    require(str(sdist_version) == version, f"mismatched version: {path.name}")
    with tarfile.open(path, "r:gz") as archive:
        prefixes = {member.name.split("/")[0] for member in archive.getmembers()}
        require(len(prefixes) == 1, "source archive must have a single root")
        prefix = prefixes.pop() + "/"

        def read(member: str) -> bytes:
            file = archive.extractfile(member)
            require(file is not None, f"missing source file: {member}")
            assert file is not None
            return file.read()

        check_metadata(
            BytesParser().parsebytes(read(prefix + "PKG-INFO")), project, version
        )
        check_files(read, prefix, project["license-files"], root)
        check_files(
            read,
            prefix,
            [
                "pyproject.toml",
                "Cargo.lock",
                "crates/python/Cargo.toml",
                "crates/python/src/lib.rs",
                "python/topology_checker/_native.pyi",
                "python/topology_checker/py.typed",
                "python/README.md",
                "tests/test_api.py",
                "tests/test_registration.py",
                "docs/python-usage.md",
                "docs/python-release.md",
            ],
            root,
        )


def checksum_manifest(archives: list[Path]) -> str:
    """Return the standard sorted SHA-256 manifest for exactly the release archives."""
    return "".join(
        f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n"
        for path in sorted(archives)
    )


def validate_release(directory: Path, root: Path = ROOT, tag: str | None = None) -> str:
    """Reject incomplete or inconsistent releases and return their checksum manifest."""
    project, version = project_config(root)
    if tag is not None:
        require(
            tag == f"python-v{version}", f"tag must be python-v{version}, found {tag}"
        )
    entries = list(directory.iterdir())
    wheels = sorted(path for path in entries if path.name.endswith(".whl"))
    sdists = sorted(path for path in entries if path.name.endswith(".tar.gz"))
    archives = wheels + sdists
    require(
        all(
            path.is_file() and path.name == "SHA256SUMS"
            for path in set(entries) - set(archives)
        ),
        "unexpected release bundle entries",
    )
    require(
        len(wheels) == 4, f"expected exactly four platform wheels, found {len(wheels)}"
    )
    require(len(sdists) == 1, f"expected one source archive, found {len(sdists)}")
    platforms = [check_wheel(path, project, version, root) for path in wheels]
    require(set(platforms) == PLATFORMS, f"missing or duplicate platforms: {platforms}")
    check_sdist(sdists[0], project, version, root)
    return checksum_manifest(archives)


def main() -> None:
    """Validate a CI bundle, then create or verify its checksum manifest."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--tag")
    parser.add_argument("--verify-checksums", action="store_true")
    args = parser.parse_args()
    try:
        manifest = validate_release(args.directory, tag=args.tag)
        destination = args.directory / "SHA256SUMS"
        if args.verify_checksums:
            require(
                destination.read_text() == manifest, "checksum manifest does not match"
            )
        else:
            destination.write_text(manifest, encoding="utf-8")
    except (
        ValueError,
        KeyError,
        OSError,
        tarfile.TarError,
        zipfile.BadZipFile,
    ) as error:
        raise SystemExit(f"Release validation failed: {error}") from error
    print("Validated four platform wheels, one source archive and SHA256SUMS")


if __name__ == "__main__":
    main()
