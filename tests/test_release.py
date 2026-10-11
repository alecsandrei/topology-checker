"""Release archive validation and executable documentation examples."""

from __future__ import annotations

import io
import re
import tarfile
import zipfile
from pathlib import Path
from typing import Any

import pytest
from shapely.geometry import LineString, box

from scripts.validate_release import (
    ROOT,
    check_native,
    checksum_manifest,
    project_config,
    validate_release,
)

TAGS = [
    "manylinux_2_17_x86_64.manylinux2014_x86_64",
    "win_amd64",
    "macosx_10_12_x86_64",
    "macosx_11_0_arm64",
]


def metadata_text(version: str = "0.1.0") -> str:
    """Create realistic metadata for the synthetic archive fixtures."""
    project, _ = project_config(ROOT)
    headers = [
        "Metadata-Version: 2.4",
        f"Name: {project['name']}",
        f"Version: {version}",
        f"Requires-Python: {project['requires-python']}",
        f"License-Expression: {project['license']}",
        "Provides-Extra: geopandas",
    ]
    headers.extend(f"Requires-Dist: {dep}" for dep in project["dependencies"])
    headers.append('Requires-Dist: geopandas>=0.14,<2; extra == "geopandas"')
    headers.extend(f"License-File: {path}" for path in project["license-files"])
    headers.extend(f"Classifier: {value}" for value in project["classifiers"])
    return "\n".join(headers) + "\n\nTest description\n"


def binary_header(tag: str) -> bytes:
    """Create just enough binary header data to test architecture validation."""
    payload = bytearray(128)
    if tag.startswith("manylinux"):
        payload[:6] = b"\x7fELF\x02\x01"
        payload[18:20] = (62).to_bytes(2, "little")
    elif tag == "win_amd64":
        payload[:2] = b"MZ"
        payload[60:64] = (64).to_bytes(4, "little")
        payload[64:68] = b"PE\0\0"
        payload[68:70] = (0x8664).to_bytes(2, "little")
    else:
        payload[:4] = b"\xcf\xfa\xed\xfe"
        machine = 0x0100000C if tag.endswith("arm64") else 0x01000007
        payload[4:8] = machine.to_bytes(4, "little")
    return bytes(payload)


def make_wheel(directory: Path, tag: str, version: str = "0.1.0") -> Path:
    """Write a synthetic wheel for structural validator tests, without executing it."""
    path = directory / f"topology_checker-{version}-cp310-abi3-{tag}.whl"
    prefix = f"topology_checker-{version}.dist-info/"
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr(prefix + "METADATA", metadata_text(version))
        tags = "\n".join(f"Tag: cp310-abi3-{part}" for part in tag.split("."))
        archive.writestr(
            prefix + "WHEEL", f"Wheel-Version: 1.0\nRoot-Is-Purelib: false\n{tags}\n"
        )
        for license_file in ["LICENSE", "THIRD-PARTY-NOTICES.txt"]:
            archive.write(ROOT / license_file, prefix + "licenses/" + license_file)
        for source in (ROOT / "python/topology_checker").iterdir():
            if source.suffix in {".py", ".pyi"} or source.name == "py.typed":
                archive.write(source, "topology_checker/" + source.name)
        extension = "_native.pyd" if tag == "win_amd64" else "_native.abi3.so"
        archive.writestr("topology_checker/" + extension, binary_header(tag))
    return path


def make_sdist(directory: Path) -> Path:
    """Package actual reviewed source files into a synthetic sdist fixture."""
    path = directory / "topology_checker-0.1.0.tar.gz"
    prefix = "topology_checker-0.1.0/"
    with tarfile.open(path, "w:gz") as archive:
        for source in [
            ROOT / "pyproject.toml",
            ROOT / "Cargo.lock",
            ROOT / "LICENSE",
            ROOT / "THIRD-PARTY-NOTICES.txt",
            *[file for file in (ROOT / "crates/python").rglob("*") if file.is_file()],
            *[file for file in (ROOT / "python").rglob("*") if file.is_file()],
            *[file for file in (ROOT / "docs").rglob("*.md") if file.is_file()],
            ROOT / "tests/test_api.py",
            ROOT / "tests/test_registration.py",
        ]:
            archive.add(source, prefix + str(source.relative_to(ROOT)))
        metadata = metadata_text().encode()
        info = tarfile.TarInfo(prefix + "PKG-INFO")
        info.size = len(metadata)
        archive.addfile(info, io.BytesIO(metadata))
    return path


@pytest.fixture
def bundle(tmp_path: Path) -> Path:
    """Provide a complete synthetic four-platform release bundle."""
    for tag in TAGS:
        make_wheel(tmp_path, tag)
    make_sdist(tmp_path)
    return tmp_path


def rewrite_wheel(
    path: Path, remove_suffix: str | None = None, crlf: bool = False
) -> None:
    """Remove one payload to verify that archive validation fails closed."""
    with zipfile.ZipFile(path) as archive:
        files = {name: archive.read(name) for name in archive.namelist()}
    with zipfile.ZipFile(path, "w") as archive:
        for name, payload in files.items():
            if remove_suffix is not None and name.endswith(remove_suffix):
                continue
            if crlf and not name.endswith((".pyd", ".so")):
                payload = payload.replace(b"\r\n", b"\n").replace(b"\n", b"\r\n")
            archive.writestr(name, payload)


def test_complete_bundle_and_checksums(bundle: Path) -> None:
    """All four platforms are mandatory and checksums cover only the archives."""
    manifest = validate_release(bundle, tag="python-v0.1.0")
    assert len(manifest.splitlines()) == 5
    assert manifest == checksum_manifest(list(bundle.iterdir()))
    (bundle / "SHA256SUMS").write_text(manifest)
    assert validate_release(bundle) == manifest
    wheel = next(bundle.glob("*.whl"))
    wheel.write_bytes(wheel.read_bytes() + b"changed")
    assert validate_release(bundle) != manifest


def test_missing_platform(bundle: Path) -> None:
    """A missing wheel prevents a release bundle from being accepted."""
    next(bundle.glob("*win_amd64.whl")).unlink()
    with pytest.raises(ValueError, match="four platform wheels"):
        validate_release(bundle)


def test_duplicate_platform(bundle: Path) -> None:
    """Four wheel files cannot substitute for four distinct platforms."""
    next(bundle.glob("*win_amd64.whl")).unlink()
    make_wheel(bundle, "macosx_10_13_x86_64")
    with pytest.raises(ValueError, match="missing or duplicate platforms"):
        validate_release(bundle)


@pytest.mark.parametrize("initial_crlf", [False, True])
def test_windows_checkout_line_endings(bundle: Path, initial_crlf: bool) -> None:
    """Windows checkout line endings do not invalidate equivalent source text."""
    wheel = next(bundle.glob("*win_amd64.whl"))
    with zipfile.ZipFile(wheel) as archive:
        files = {name: archive.read(name) for name in archive.namelist()}
    with zipfile.ZipFile(wheel, "w") as archive:
        for name, payload in files.items():
            if not name.endswith(".pyd"):
                payload = payload.replace(b"\r\n", b"\n")
            archive.writestr(name, payload)
    if initial_crlf:
        rewrite_wheel(wheel, crlf=True)
    rewrite_wheel(wheel, crlf=True)
    with zipfile.ZipFile(wheel) as archive:
        before = {name: archive.read(name) for name in archive.namelist()}
    rewrite_wheel(wheel, crlf=True)
    with zipfile.ZipFile(wheel) as archive:
        assert before == {name: archive.read(name) for name in archive.namelist()}
    assert len(validate_release(bundle).splitlines()) == 5


def test_mismatched_version(bundle: Path) -> None:
    """Wheel versions must agree with the binding version and source archive."""
    next(bundle.glob("*win_amd64.whl")).unlink()
    make_wheel(bundle, "win_amd64", "0.2.0")
    with pytest.raises(ValueError, match="mismatched version"):
        validate_release(bundle)


@pytest.mark.parametrize(
    "suffix",
    [
        "licenses/LICENSE",
        "licenses/THIRD-PARTY-NOTICES.txt",
        "py.typed",
        "_native.pyi",
        "_native.pyd",
    ],
)
def test_missing_required_files(bundle: Path, suffix: str) -> None:
    """License texts, typing data and the extension must be packaged."""
    rewrite_wheel(next(bundle.glob("*win_amd64.whl")), suffix)
    with pytest.raises((ValueError, KeyError)):
        validate_release(bundle)


@pytest.mark.parametrize(
    "tag", ["linux_x86_64", "manylinux_2_28_x86_64", "manylinux2014_aarch64"]
)
def test_invalid_linux_tag(bundle: Path, tag: str) -> None:
    """Reject generic Linux, newer glibc baselines and unsupported architectures."""
    path = next(bundle.glob("*manylinux*.whl"))
    path.rename(bundle / f"topology_checker-0.1.0-cp310-abi3-{tag}.whl")
    with pytest.raises(ValueError, match="unsupported platform tag"):
        validate_release(bundle)


def test_wrong_tag_and_unexpected_bundle_files(bundle: Path) -> None:
    """Release tags must match the version, and the bundle cannot contain extras."""
    with pytest.raises(ValueError, match="tag must be python-v0.1.0"):
        validate_release(bundle, tag="python-v0.2.0")
    (bundle / "extra.txt").write_text("not a release archive")
    with pytest.raises(ValueError, match="unexpected release bundle entries"):
        validate_release(bundle)


def test_wrong_binary_architecture() -> None:
    """An Apple Silicon binary must not be labeled as an Intel wheel."""
    with pytest.raises(ValueError, match="native extension does not match"):
        check_native(binary_header("macosx_11_0_arm64"), "macos-x86_64")


@pytest.mark.parametrize(
    "document", ["README.md", "python/README.md", "docs/python-usage.md"]
)
def test_documentation_examples(document: str, monkeypatch: pytest.MonkeyPatch) -> None:
    """Execute each documented API block with in-memory GeoPandas input."""
    gpd = pytest.importorskip("geopandas")
    roads = gpd.GeoDataFrame(geometry=[LineString([(0, 0), (1, 0)])], crs=28992)
    buildings = gpd.GeoDataFrame(geometry=[box(0, 0, 1, 1)], crs=28992)

    def read_file(path: str) -> Any:
        return roads if path == "roads.gpkg" else buildings

    monkeypatch.setattr(gpd, "read_file", read_file)
    namespace: dict[str, Any] = {"roads_gdf": roads}
    text = (ROOT / document).read_text()
    for block in re.findall(r"```python\n(.*?)```", text, re.DOTALL):
        exec(compile(block, document, "exec"), namespace)
