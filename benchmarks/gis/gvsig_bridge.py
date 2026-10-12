"""Jython startup addon driving gvSIG's stock Topology Framework."""

import json
import os
import threading
import time
import traceback

from java.lang import System, Throwable

DIRECTORY = os.environ["GIS_WORKER_DIRECTORY"]


def reply(value):
    """Publish a response outside the validation clock."""
    temporary = os.path.join(DIRECTORY, "response.tmp")
    with open(temporary, "w") as stream:
        json.dump(value, stream)
    os.rename(temporary, os.path.join(DIRECTORY, "response.json"))


def phase(value):
    """Signal a separate validation deadline."""
    temporary = os.path.join(DIRECTORY, "phase.tmp")
    with open(temporary, "w") as stream:
        stream.write(value)
    os.rename(temporary, os.path.join(DIRECTORY, "phase.txt"))


def wkt(geometry):
    """Serialize fixture coordinates for gvSIG's native geometry parser."""

    def coordinates(value):
        if isinstance(value[0], (int, float)):
            return " ".join(str(v) for v in value[:2])
        return "(" + ",".join(coordinates(child) for child in value) + ")"

    kind = geometry["type"]
    value = geometry["coordinates"]
    if kind == "Point":
        return "POINT (" + coordinates(value) + ")"
    return kind.upper() + " " + coordinates(value)


def fresh_store(manager, geometries):
    """Copy preloaded native geometries into a fresh in-memory feature store."""
    from org.gvsig.fmap.dal import DataTypes
    from org.gvsig.fmap.geom import Geometry

    store = manager.createMemoryStore(None)
    store.edit()
    schema = manager.createFeatureType()
    schema.add("bench_id", DataTypes.INT)
    attribute = schema.add("GEOMETRY", DataTypes.GEOMETRY)
    schema.setDefaultGeometryAttributeName("GEOMETRY")
    attribute.setGeometryType(geometries[0][1].getType(), Geometry.SUBTYPES.GEOM2D)
    store.update(schema)
    for fid, geometry in geometries:
        feature = store.createNewFeature()
        feature.set("bench_id", fid)
        feature.set("GEOMETRY", geometry.cloneGeometry())
        store.insert(feature)
    store.finishEditing()
    return store


def worker():
    """Run the actual plan including its lazy spatial indexes and report geometries."""
    try:
        from gvsig import uselib

        uselib.use_plugin("org.gvsig.topology.app.mainplugin")
        from org.gvsig.app import ApplicationLocator
        from org.gvsig.fmap.geom import GeometryLocator
        from org.gvsig.topology.lib.api import TopologyLocator

        manager = ApplicationLocator.getManager().getDataManager()
        geometry_manager = GeometryLocator.getGeometryManager()
        topology_manager = TopologyLocator.getTopologyManager()
        files = json.loads(os.environ["GIS_WORKER_FIXTURE"])
        sources = {}
        for name, path in files.items():
            with open(path) as stream:
                features = json.load(stream)["features"]
            sources[name] = [
                (
                    int(f["properties"]["bench_id"]),
                    geometry_manager.createFrom(wkt(f["geometry"])),
                )
                for f in features
            ]
        available = [
            str(factory.getId()) for factory in topology_manager.getRuleFactories()
        ]
        mappings = json.loads(os.environ["GIS_WORKER_MAPPINGS"])
        rule = os.environ["GIS_WORKER_RULE"]
        aliases = {
            "point/must-be-inside": ["PointMustBeProperlyInsidePolygon", mappings[rule]]
            if rule == "point/must-be-inside"
            else [],
            "polygon/must-contain-point": ["PolygonContainsPoint", mappings[rule]]
            if rule == "polygon/must-contain-point"
            else [],
            "polygon/must-not-overlap": ["PolygonMustNotOverlapPolygon", mappings[rule]]
            if rule == "polygon/must-not-overlap"
            else [],
        }
        candidates = aliases.get(rule) or [mappings[rule]]
        factory_id = next((value for value in candidates if value in available), None)
        if factory_id is None:
            reply(
                {
                    "status": "failed",
                    "error": "Mapped rule absent from installed gvSIG: " + rule,
                    "available_rules": available,
                }
            )
            return
        reply(
            {
                "status": "ready",
                "version": "2.7.0-5016",
                "factory": factory_id,
                "available_rules": available,
                "java": System.getProperty("java.version"),
                "workflow": "stock TopologyPlan, UI disabled",
            }
        )
        while True:
            request = os.path.join(DIRECTORY, "request.json")
            if not os.path.exists(request):
                time.sleep(0.02)
                continue
            stores = []
            try:
                with open(request) as stream:
                    command = json.load(stream)
                os.unlink(request)
                if command["rule"] != rule:
                    raise ValueError("Session rule cannot change")
                plan = topology_manager.createTopologyPlan()
                plan.setUseUI(False)
                plan.setAcceptExceptions(False)
                for name in ("input", "other"):
                    if name not in sources:
                        continue
                    store = fresh_store(manager, sources[name])
                    stores.append(store)
                    plan.addDataSet(name, store)
                phase("validating")
                started = System.nanoTime()
                plan.addRule(
                    factory_id, "input", "other" if "other" in sources else None, 0.0
                )
                plan.execute()
                seconds = (System.nanoTime() - started) / 1000000000.0
                phase("extracting")
                errors = []
                for line in plan.getReport().getLines():
                    if line.isException():
                        raise RuntimeError(
                            "gvSIG reported an execution exception: "
                            + str(line.getDescription())
                        )
                    reference = line.getFeature1()
                    fid = (
                        None
                        if reference is None
                        else int(reference.getFeature().get("bench_id"))
                    )
                    errors.append({"kind": "violation", "id": fid, "layer": 0})
                reply(
                    {
                        "status": "ok",
                        "seconds": seconds,
                        "raw_count": len(errors),
                        "errors": errors,
                    }
                )
            except (Exception, Throwable):
                reply({"status": "failed", "error": traceback.format_exc()})
            finally:
                for store in stores:
                    store.dispose()
    except (Exception, Throwable):
        reply({"status": "failed", "error": traceback.format_exc()})


def main(*args):
    """Start the worker without blocking gvSIG's Swing event thread."""
    thread = threading.Thread(target=worker, name="GIS benchmark worker")
    thread.daemon = True
    thread.start()
