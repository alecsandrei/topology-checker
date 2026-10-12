"""Embedded GRASS worker, executed with GRASS's system Python environment."""

import json
import os
import shutil
import time
import traceback
from pathlib import Path

import grass.script as gs
from grass.exceptions import CalledModuleError

import grass_native

DIRECTORY = Path(os.environ["GIS_WORKER_DIRECTORY"])
SOURCE_FILES = json.loads(os.environ["GIS_WORKER_FIXTURE"])
MAP_TYPES = {}
SELECTIONS = {
    "point/must-be-inside": ("within", True),
    "point/must-be-covered-by-lines": ("intersects", True),
    "point/must-be-covered-by-polygons": ("intersects", True),
    "point/must-not-be-inside-polygons": ("within", False),
    "point/must-not-intersect-lines": ("intersects", False),
    "point/must-not-intersect-polygons": ("intersects", False),
    "polygon/must-contain-point": ("contains", True),
    "polygon/must-not-contain-points": ("contains", False),
}


def reply(value: dict) -> None:
    """Publish an atomic IPC response outside the measured operation."""
    temporary = DIRECTORY / "response.tmp"
    temporary.write_text(json.dumps(value))
    temporary.replace(DIRECTORY / "response.json")


def phase(value: str) -> None:
    """Signal a separate validation deadline."""
    temporary = DIRECTORY / "phase.tmp"
    temporary.write_text(value)
    temporary.replace(DIRECTORY / "phase.txt")


def rows(map_name: str) -> list[dict]:
    """Recover preserved benchmark IDs from the result map's category table."""
    # v.select produces no vector map when the selected set is empty.
    if not gs.find_file(map_name, element="vector")["name"]:
        return []
    connection = gs.vector_db(map_name).get(1)
    if connection is None:
        info = gs.vector_info_topo(map_name)
        count = info.get("areas", 0) or (info.get("points", 0) + info.get("lines", 0))
        return [{"kind": "violation", "id": None, "layer": 0} for _ in range(count)]
    selected = gs.vector_db_select(map_name, columns="bench_id")
    return [
        {"kind": "violation", "id": int(row[0]), "layer": 0}
        for row in selected["values"].values()
    ]


def main() -> None:
    """Import uncleaned templates, then sample with freshly built topology."""
    try:
        for name, path in SOURCE_FILES.items():
            kind = json.loads(Path(path).read_text())["features"][0]["geometry"]["type"]
            MAP_TYPES[name] = (
                "area" if "Polygon" in kind else "point" if "Point" in kind else "line"
            )
            gs.run_command(
                "v.in.ogr",
                input=path,
                output="source_" + name,
                flags="c",
                snap=-1,
                min_area=0,
                quiet=True,
            )
        polygon_names = []
        coverage_rings = {}
        if os.environ["GIS_WORKER_RULE"] == "polygon/must-not-overlap":
            features = json.loads(Path(SOURCE_FILES["input"]).read_text())["features"]
            polygon_names = grass_native.polygon_templates(features)
        if os.environ["GIS_WORKER_RULE"] == "polygon/must-not-have-gaps":
            features = json.loads(Path(SOURCE_FILES["input"]).read_text())["features"]
            coverage_rings = grass_native.coverage_templates(features)
        reply(
            {
                "status": "ready",
                "version": gs.version()["version"],
                "workflow": "GRASS vector modules",
            }
        )
    except (CalledModuleError, RuntimeError, OSError, ValueError):
        reply({"status": "failed", "error": traceback.format_exc()})
        return
    while True:
        request = DIRECTORY / "request.json"
        if not request.exists():
            time.sleep(0.02)
            continue
        try:
            command = json.loads(request.read_text())
            request.unlink()
            rule = command["rule"]
            gs.run_command(
                "g.remove",
                flags="f",
                type="vector",
                name="input,other,errors,clean,noded,faces",
                quiet=True,
            )
            gs.run_command(
                "g.remove", flags="f", type="vector", pattern="overlap_*", quiet=True
            )
            env = gs.gisenv()
            for name in SOURCE_FILES:
                gs.run_command(
                    "g.copy", vector="source_" + name + "," + name, quiet=True
                )
                # Copies retain native coordinates and categories. Drop only support
                # files so index construction is measured, not hidden in import.
                vector = (
                    Path(env["GISDBASE"])
                    / env["LOCATION_NAME"]
                    / env["MAPSET"]
                    / "vector"
                    / name
                )
                for support in ("topo", "sidx", "cidx", "fidx"):
                    (vector / support).unlink(missing_ok=True)
            for template in (
                ["source_index", *polygon_names]
                if polygon_names or coverage_rings
                else []
            ):
                target = template.replace("source_", "")
                root = vector.parent
                shutil.rmtree(root / target, ignore_errors=True)
                shutil.copytree(root / template, root / target)
                for support in ("topo", "sidx", "cidx", "fidx"):
                    (root / target / support).unlink(missing_ok=True)
            phase("validating")
            started = time.perf_counter()
            for name in SOURCE_FILES:
                gs.run_command("v.build", map=name, quiet=True)
            if rule in SELECTIONS:
                operator, reverse = SELECTIONS[rule]
                gs.run_command(
                    "v.select",
                    ainput="input",
                    binput="other",
                    atype=MAP_TYPES["input"],
                    btype=MAP_TYPES["other"],
                    output="errors",
                    operator=operator,
                    flags="r" if reverse else "",
                    quiet=True,
                )
            elif rule == "point/must-not-overlap":
                materialized = grass_native.duplicates()
            elif rule == "line/must-not-have-dangles":
                materialized = grass_native.dangles()
            elif rule == "polygon/must-not-overlap":
                materialized = grass_native.overlaps(polygon_names)
            elif rule == "polygon/must-not-have-gaps":
                materialized = grass_native.gaps(coverage_rings)
            else:
                raise ValueError("unsupported native GRASS rule")
            seconds = time.perf_counter() - started
            phase("extracting")
            errors = (
                rows("errors")
                if rule in SELECTIONS
                else grass_native.extract(materialized)
            )
            reply(
                {
                    "status": "ok",
                    "seconds": seconds,
                    "raw_count": len(errors),
                    "errors": errors,
                }
            )
        except (CalledModuleError, RuntimeError, OSError, ValueError, KeyError):
            reply({"status": "failed", "error": traceback.format_exc()})


if __name__ == "__main__":
    main()
