"""Build the RustGIS plugin and package it as `topology-checker-<version>.rgplugin`.

A package is a zip with `plugin.json` and the WebAssembly module at its root;
install it from RustGIS with Plugin Manager > Install from File. Usage:

    python scripts/package_rustgis_plugin.py [--out DIR] [--wasm FILE] [--notices FILE]

`--wasm` packages an existing module instead of building one. `--notices` adds a
third-party notices file.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
import zipfile
from pathlib import Path
from typing import Any, cast

ROOT = Path(__file__).resolve().parents[1]
CRATE = "topology-checker-rustgis"
MANIFEST = ROOT / "crates" / "rustgis-plugin" / "plugin.json"
BUILT_WASM = (
    ROOT
    / "target"
    / "wasm32-unknown-unknown"
    / "rustgis-plugin"
    / "topology_checker_rustgis.wasm"
)

# Limits from the RustGIS plugin documentation (docs/plugins.md).
MAX_PACKAGE_BYTES = 64 * 1024 * 1024
MAX_MODULE_BYTES = 48 * 1024 * 1024
ABI = 1
ID_PATTERN = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*$")
VERSION_PATTERN = re.compile(r"^\d+\.\d+\.\d+$")
# Fixed timestamp so the same inputs give the same package.
ZIP_DATE = (1980, 1, 1, 0, 0, 0)


def build_module() -> Path:
    """Compile the plugin for wasm32 with the size-tuned workspace profile."""
    subprocess.run(
        [
            "cargo",
            "build",
            "--profile",
            "rustgis-plugin",
            "--target",
            "wasm32-unknown-unknown",
            "-p",
            CRATE,
            "--locked",
        ],
        cwd=ROOT,
        check=True,
    )
    return BUILT_WASM


def check_manifest(manifest: dict[str, Any]) -> list[str]:
    """Return the problems with a manifest, judged by the documented RustGIS rules."""
    problems: list[str] = []
    if not ID_PATTERN.match(str(manifest.get("id", ""))):
        problems.append("id must be lower-case letters, digits and hyphens")
    for key in ("version", "minRustgisVersion"):
        if not VERSION_PATTERN.match(str(manifest.get(key, ""))):
            problems.append(f"{key} must look like 1.2.3")
    if manifest.get("abi") != ABI:
        problems.append(f"abi must be {ABI}")
    if manifest.get("entry") != "plugin.wasm":
        problems.append("entry must be plugin.wasm")
    tools = cast(list[dict[str, Any]], manifest.get("tools") or [])
    ids = [str(t["id"]) for t in tools]
    if not tools or len(set(ids)) != len(ids):
        problems.append("tools must be present and have distinct ids")
    ribbon: list[str] = []
    tab = cast(dict[str, Any], manifest.get("tab") or {})
    groups = cast(list[dict[str, Any]], tab.get("groups") or [])
    if not 1 <= len(groups) <= 8:
        problems.append("a tab has 1-8 groups")
    for group in groups:
        items = cast("list[str | dict[str, Any]]", group["items"])
        if not 1 <= len(items) <= 16:
            problems.append(f"group {group['label']!r} must have 1-16 items")
        for item in items:
            if isinstance(item, str):
                ribbon.append(item)
                continue
            entries = cast(list[str], item["items"])
            if not 1 <= len(entries) <= 32:
                problems.append(f"menu {item['menu']!r} must have 1-32 entries")
            ribbon += entries
    unknown = [e for e in ribbon if e.removeprefix("tool:") not in ids]
    if unknown:
        problems.append(f"ribbon names things the plugin lacks: {unknown}")
    return problems


def package(wasm: Path, out_dir: Path, notices: Path | None) -> Path:
    """Write the .rgplugin and return its path."""
    manifest = cast(dict[str, Any], json.loads(MANIFEST.read_text()))
    problems = check_manifest(manifest)
    if wasm.stat().st_size > MAX_MODULE_BYTES:
        problems.append(
            f"module is {wasm.stat().st_size} bytes, over {MAX_MODULE_BYTES}"
        )
    if problems:
        raise SystemExit("plugin.json: " + "; ".join(problems))
    out_dir.mkdir(parents=True, exist_ok=True)
    target = out_dir / f"{manifest['id']}-{manifest['version']}.rgplugin"
    files = {
        "plugin.json": MANIFEST.read_bytes(),
        manifest["entry"]: wasm.read_bytes(),
        "LICENSE": (ROOT / "LICENSE").read_bytes(),
    }
    if notices is not None:
        files["THIRD-PARTY-NOTICES.txt"] = notices.read_bytes()
    with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for name, data in files.items():
            info = zipfile.ZipInfo(name, ZIP_DATE)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            z.writestr(info, data)
    if target.stat().st_size > MAX_PACKAGE_BYTES:
        raise SystemExit(f"{target.name} is over {MAX_PACKAGE_BYTES} bytes")
    return target


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Build and package the RustGIS plugin."
    )
    parser.add_argument("--out", type=Path, default=ROOT / "dist")
    parser.add_argument(
        "--wasm", type=Path, help="package this module instead of building one"
    )
    parser.add_argument("--notices", type=Path, help="third-party notices to include")
    args = parser.parse_args()
    wasm = cast("Path | None", args.wasm) or build_module()
    target = package(wasm, cast(Path, args.out), cast("Path | None", args.notices))
    digest = hashlib.sha256(target.read_bytes()).hexdigest()
    print(f"{target}  {target.stat().st_size} bytes  sha256 {digest}")


if __name__ == "__main__":
    sys.exit(main())
