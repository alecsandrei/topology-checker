"""Drive the installed C++ plugin through its Qt rule table and Validate All slot.

Executed by QGIS's embedded interpreter, which supplies qgis and PyQt.
"""

import hashlib
import json
import os
import time
import traceback
from pathlib import Path

from qgis.core import Qgis, QgsApplication, QgsFeature, QgsProject, QgsVectorLayer
from qgis.PyQt.QtCore import PYQT_VERSION_STR, QT_VERSION_STR, QMetaObject, Qt, QTimer
from qgis.PyQt.QtGui import QAction
from qgis.PyQt.QtWidgets import (
    QApplication,
    QComboBox,
    QDockWidget,
    QTableView,
    QTableWidget,
    QTableWidgetItem,
)
from qgis.utils import iface

DIRECTORY = Path(os.environ["TOPOLOGY_QGIS_SESSION"])
LAYERS: list = []
SOURCES: list = []
BUSY = False


def reply(value: dict) -> None:
    """Atomically publish a response outside validation timing."""
    temporary = DIRECTORY / "response.tmp"
    temporary.write_text(json.dumps(value))
    temporary.replace(DIRECTORY / "response.json")


def phase(name: str) -> None:
    """Signal watchdog phases outside the validation clock."""
    temporary = DIRECTORY / "phase.tmp"
    temporary.write_text(name)
    temporary.replace(DIRECTORY / "phase.txt")


def load(path: str, name: str) -> tuple:
    """Preload OGR features; create a fresh memory layer for every sample."""
    source = QgsVectorLayer(path, name, "ogr")
    if not source.isValid():
        raise ValueError(f"Invalid fixture: {path}")
    features = list(source.getFeatures())
    ids = [int(f["bench_id"]) for f in features]
    if ids != list(range(1, len(features) + 1)):
        raise ValueError("Fixture IDs must be sequential positive integers")
    return source, features, ids


def fresh(source: tuple, name: str) -> QgsVectorLayer:
    """Copy preloaded geometries, then verify actual memory-provider IDs."""
    original, features, ids = source
    layer = QgsVectorLayer(f"{original.wkbType().name}?crs=EPSG:3857", name, "memory")
    # Qt enum names can be WKB-prefixed; use the canonical QGIS type string.
    if not layer.isValid():
        from qgis.core import QgsWkbTypes

        layer = QgsVectorLayer(
            f"{QgsWkbTypes.displayString(original.wkbType())}?crs=EPSG:3857",
            name,
            "memory",
        )
    copies = []
    for feature in features:
        copy = QgsFeature()
        copy.setGeometry(feature.geometry())
        copies.append(copy)
    ok, added = layer.dataProvider().addFeatures(copies)
    if not ok or [f.id() for f in added] != ids:
        raise ValueError("Memory-provider ID mapping changed")
    QgsProject.instance().addMapLayer(layer, False)
    return layer


def handle() -> None:
    """Process at most one request and avoid reentry from plugin progress events."""
    global BUSY, SOURCES, LAYERS
    request = DIRECTORY / "request.json"
    if BUSY or not request.exists():
        return
    BUSY = True
    try:
        command = json.loads(request.read_text())
        request.unlink()
        if command["action"] == "load":
            SOURCES = [load(command["input"], "input")]
            if command.get("other"):
                SOURCES.append(load(command["other"], "other"))
            reply({"status": "ready", "version": Qgis.QGIS_VERSION})
        elif command["action"] == "sample":
            QMetaObject.invokeMethod(
                DOCK, "deleteErrors", Qt.ConnectionType.DirectConnection
            )
            for layer in LAYERS:
                QgsProject.instance().removeMapLayer(layer.id())
            LAYERS = [
                fresh(s, name)
                for s, name in zip(SOURCES, ["input", "other"], strict=False)
            ]
            TABLE.setRowCount(1)
            for col, value in enumerate(
                [
                    command["rule"],
                    "input",
                    "other" if len(LAYERS) == 2 else "",
                    LAYERS[0].id(),
                    LAYERS[1].id() if len(LAYERS) == 2 else "",
                ]
            ):
                item = QTableWidgetItem(value)
                if col == 0:
                    item.setCheckState(Qt.CheckState.Checked)
                TABLE.setItem(0, col, item)
            for widget in QApplication.allWidgets():
                widget.setUpdatesEnabled(False)
            phase("validating")
            start = time.perf_counter()
            QMetaObject.invokeMethod(
                DOCK, "validateAll", Qt.ConnectionType.DirectConnection
            )
            seconds = time.perf_counter() - start
            phase("extracting")
            model = DOCK.findChild(QTableView, "mErrorTableView").model()
            rows = [
                {
                    "kind": model.data(model.index(row, 0)),
                    "layer": model.data(model.index(row, 1)),
                    "id": int(model.data(model.index(row, 2))),
                }
                for row in range(model.rowCount())
            ]
            reply(
                {
                    "status": "ok",
                    "seconds": seconds,
                    "raw_count": len(rows),
                    "errors": rows,
                }
            )
        else:
            raise ValueError("Unknown request")
    except (ValueError, RuntimeError, OSError, AttributeError, TypeError):
        reply({"status": "failed", "error": traceback.format_exc()})
    finally:
        BUSY = False


try:
    WINDOW = iface.mainWindow()
    iface.mapCanvas().freeze(True)
    iface.mapCanvas().setRenderFlag(False)
    for action in WINDOW.findChildren(QAction):
        if action.text() == "Topology Checker":
            action.trigger()
            break
    DOCK = WINDOW.findChild(QDockWidget, "checkDock")
    TABLE = next(
        t
        for w in QApplication.topLevelWidgets()
        for t in w.findChildren(QTableWidget)
        if t.objectName() == "mRulesTable"
    )
    AVAILABLE = next(
        w.findChild(QComboBox, "mRuleBox")
        for w in QApplication.topLevelWidgets()
        if w.findChild(QComboBox, "mRuleBox") is not None
    )
    WINDOW.setUpdatesEnabled(False)
    plugin = Path(QgsApplication.pluginPath()) / "libplugin_topology.so"
    reply(
        {
            "status": "ready",
            "version": Qgis.QGIS_VERSION,
            "rules": [AVAILABLE.itemText(i) for i in range(AVAILABLE.count())],
            "qt": QT_VERSION_STR,
            "pyqt": PYQT_VERSION_STR,
            "revision": Qgis.QGIS_DEV_VERSION,
            "plugin_path": str(plugin),
            "plugin_sha256": hashlib.sha256(plugin.read_bytes()).hexdigest()
            if plugin.is_file()
            else None,
        }
    )
    TIMER = QTimer()
    TIMER.timeout.connect(handle)
    TIMER.start(20)
except (ValueError, RuntimeError, OSError, AttributeError, StopIteration):
    reply({"status": "failed", "error": traceback.format_exc()})
