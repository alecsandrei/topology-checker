"""QGIS native geometry workflows when bundled rules have different semantics."""

import json
import os
import time
import traceback
from pathlib import Path

from qgis.core import (
    Qgis,
    QgsApplication,
    QgsFeature,
    QgsGeometry,
    QgsPointXY,
    QgsSpatialIndex,
    QgsVectorLayer,
)

DIRECTORY = Path(os.environ["GIS_WORKER_DIRECTORY"])


def reply(value: dict) -> None:
    """Serialize after validation."""
    temporary = DIRECTORY / "response.tmp"
    temporary.write_text(json.dumps(value))
    temporary.replace(DIRECTORY / "response.json")


def phase(value: str) -> None:
    """Signal a watchdog phase outside timing."""
    (DIRECTORY / "phase.txt").write_text(value)


def validate(rule: str, geometries: dict) -> list:
    """Build a fresh native spatial index and materialize native error geometries."""
    features = []
    for fid, geometry in geometries.items():
        feature = QgsFeature(fid)
        feature.setGeometry(geometry)
        features.append(feature)
    index = QgsSpatialIndex()
    index.addFeatures(features)
    errors = []
    if rule == "polygon/must-not-have-gaps":
        merged = QgsGeometry.unaryUnion(list(geometries.values()))
        polygons = (
            merged.asMultiPolygon() if merged.isMultipart() else [merged.asPolygon()]
        )
        for polygon in polygons:
            for ring in polygon[1:]:
                void = QgsGeometry.fromPolygonXY([ring]).difference(merged)
                for part in (
                    void.asGeometryCollection() if void.isMultipart() else [void]
                ):
                    if part.area() > 0:
                        errors.append((None, None, part))
        return errors
    for fid, geometry in geometries.items():
        if rule == "line/must-not-have-dangles":
            coordinates = geometry.asPolyline()
            endpoints = [(p.x(), p.y()) for p in (coordinates[0], coordinates[-1])]
            if endpoints[0] == endpoints[-1]:
                continue
            for xy in (coordinates[0], coordinates[-1]):
                endpoint = QgsGeometry.fromPointXY(QgsPointXY(xy))
                uses = 0
                for candidate in index.intersects(endpoint.boundingBox()):
                    other = geometries[candidate]
                    if not other.intersects(endpoint):
                        continue
                    vertices = other.asPolyline()
                    ends = [(p.x(), p.y()) for p in (vertices[0], vertices[-1])]
                    uses += (
                        2 if ends[0] == ends[-1] or (xy.x(), xy.y()) not in ends else 1
                    )
                if uses == 1:
                    # An open line can connect its endpoint to its own interior.
                    for position, (first, last) in enumerate(
                        zip(coordinates, coordinates[1:], strict=False)
                    ):
                        if (position == 0 and (xy.x(), xy.y()) == endpoints[0]) or (
                            position == len(coordinates) - 2
                            and (xy.x(), xy.y()) == endpoints[-1]
                        ):
                            continue
                        segment = QgsGeometry.fromPolylineXY([first, last])
                        if segment.length() > 0 and segment.intersects(endpoint):
                            uses += 2
                            break
                if uses == 1:
                    errors.append((fid, None, endpoint))
            continue
        for candidate in index.intersects(geometry.boundingBox()):
            if candidate <= fid:
                continue
            other = geometries[candidate]
            if rule == "point/must-not-overlap":
                if geometry.isTopologicallyEqual(other):
                    errors.append((fid, candidate, QgsGeometry(geometry)))
            else:
                intersection = geometry.intersection(other)
                if intersection.area() > 0:
                    errors.append((fid, candidate, intersection))
    return errors


def main() -> None:
    """Preload native geometries and construct fresh validation state per sample."""
    application = QgsApplication([], False)
    application.initQgis()
    if not Qgis.QGIS_VERSION.startswith("4.2."):
        raise ValueError("The suite requires QGIS 4.2.x")
    files = json.loads(os.environ["GIS_WORKER_FIXTURE"])
    layer = QgsVectorLayer(files["input"], "input", "ogr")
    if not layer.isValid():
        raise ValueError("Invalid source layer")
    source = {
        int(feature["bench_id"]): feature.geometry() for feature in layer.getFeatures()
    }
    reply(
        {
            "status": "ready",
            "version": Qgis.QGIS_VERSION,
            "workflow": "native indexed QGIS geometry operations",
        }
    )
    while True:
        request = DIRECTORY / "request.json"
        if not request.exists():
            time.sleep(0.02)
            continue
        try:
            command = json.loads(request.read_text())
            request.unlink()
            geometries = {
                fid: QgsGeometry(geometry) for fid, geometry in source.items()
            }
            phase("validating")
            started = time.perf_counter()
            materialized = validate(command["rule"], geometries)
            seconds = time.perf_counter() - started
            phase("extracting")
            errors = [
                {
                    "kind": "violation",
                    "id": fid,
                    "destination": destination,
                    "layer": 0,
                    "geometry": json.loads(geometry.asJson(17)),
                }
                for fid, destination, geometry in materialized
            ]
            reply(
                {
                    "status": "ok",
                    "seconds": seconds,
                    "raw_count": len(errors),
                    "errors": errors,
                }
            )
        except (RuntimeError, OSError, ValueError, KeyError, AttributeError, TypeError):
            reply({"status": "failed", "error": traceback.format_exc()})


if __name__ == "__main__":
    main()
