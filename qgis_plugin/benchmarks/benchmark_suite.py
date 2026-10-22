#!/usr/bin/env python3
"""Comprehensive Performance Benchmark Suite for Topology Validation:
1. QGIS Built-in C++ Topology Checker (libplugin_topology.so)
2. Topology Rules Plugin - CLI Mode (detailed stage breakdown)
3. Topology Rules Plugin - Native In-Memory Mode (detailed stage breakdown)
4. Standalone Raw Rust Topology Checker Engine

Evaluated on:
- Real-World River Network: /tmp/cursuriaparauri/CursuriApaRauri.shp (4,917 features, 15.54M vertices, 237 MB)
- Synthetic Point datasets: 1,000, 10,000, 50,000 features (point/must-not-overlap)
- Synthetic Polygon datasets: 1,000, 10,000, 50,000 features (polygon/must-not-overlap)
"""

from __future__ import annotations

import ctypes
import json
import math
import os
import subprocess
import sys
import tempfile
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

PLUGIN_DIR = Path('/Users/alex-andrei.cuvuliuc/Desktop/tc-qgis-hybrid/qgis_plugin')
sys.path.insert(0, str(PLUGIN_DIR))
sys.path.append(
    '/Applications/QGIS-final-4_2_2.app/Contents/Resources/qgis/python/plugins'
)

from qgis.core import (
    Qgis,
    QgsApplication,
    QgsCoordinateReferenceSystem,
    QgsFeature,
    QgsFeatureRequest,
    QgsGeometry,
    QgsPointXY,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsProject,
    QgsVectorFileWriter,
    QgsVectorLayer,
)
from qgis.gui import QgisInterface, QgsMapCanvas
from qgis.PyQt import sip
from qgis.PyQt.QtWidgets import QAction, QApplication

import shapely
import topology_checker
from topology_rules.core.binary import Rule
from topology_rules.core.commands import Check, rule_counts, run_validate, write_config
from topology_rules.core.inputs import export_dataset, invalid_reason
from topology_rules.core.layers import Input, wkb_kind
from topology_rules.core.native_runner import decompose_geom, resolve_rule

CLI_EXECUTABLE = '/Users/alex-andrei.cuvuliuc/Desktop/topology-checker/target/release/topology-checker'
LIBPLUGIN_TOPOLOGY = '/Applications/QGIS-final-4_2_2.app/Contents/PlugIns/qgis/libplugin_topology.so'


class HeadlessQgisIface(QgisInterface):
    """Minimal QgisInterface subclass to host the C++ Topol plugin and checkDock."""

    def __init__(self, canvas: QgsMapCanvas):
        super().__init__()
        self.canvas = canvas
        self.action: QAction | None = None
        self.dock: Any = None

    def mapCanvas(self) -> QgsMapCanvas:
        return self.canvas

    def addVectorToolBarIcon(self, action: QAction) -> int:
        self.action = action
        return 0

    def addPluginToVectorMenu(self, name: str, action: QAction) -> None:
        pass

    def addDockWidget(self, area: Any, dock: Any) -> None:
        self.dock = dock


def create_and_run_qgis_builtin_check(
    project: QgsProject,
    canvas: QgsMapCanvas,
    layer: QgsVectorLayer,
    rule_name: str,
) -> float:
    """Configures QgsProject first, then instantiates C++ Topol plugin dock and validates."""
    project.writeEntry('Topol', '/testCount', 1)
    project.writeEntry('Topol', '/testenabled_0', True)
    project.writeEntry('Topol', '/testname_0', rule_name)
    project.writeEntry('Topol', '/layer1_0', layer.id())
    project.writeEntry('Topol', '/layer2_0', '')

    iface = HeadlessQgisIface(canvas)
    ptr = sip.unwrapinstance(iface)

    lib = ctypes.CDLL(LIBPLUGIN_TOPOLOGY)
    lib.classFactory.argtypes = [ctypes.c_void_p]
    lib.classFactory.restype = ctypes.c_void_p

    plugin = lib.classFactory(ptr)
    vtable_ptr = ctypes.cast(plugin, ctypes.POINTER(ctypes.c_void_p))[0]
    vtable = ctypes.cast(vtable_ptr, ctypes.POINTER(ctypes.c_void_p))

    initGui_func = ctypes.CFUNCTYPE(None, ctypes.c_void_p)(vtable[2])
    initGui_func(plugin)
    assert iface.action is not None, 'Action not set'
    iface.action.trigger()
    assert iface.dock is not None, 'Dock not created'

    validate_action = [
        a
        for a in iface.dock.findChildren(QAction)
        if a.objectName() == 'actionValidateAll'
    ][0]

    t0 = time.perf_counter()
    validate_action.trigger()
    t1 = time.perf_counter()

    iface.dock.deleteLater()
    return (t1 - t0) * 1000.0


@dataclass
class CLIBreakdown:
    total_ms: float = 0.0
    feature_inspect_ms: float = 0.0
    gpkg_export_ms: float = 0.0
    subprocess_exec_ms: float = 0.0
    result_ingest_ms: float = 0.0
    error_count: int = 0


@dataclass
class NativeBreakdown:
    total_ms: float = 0.0
    wkb_extract_ms: float = 0.0
    rust_validate_ms: float = 0.0
    result_convert_ms: float = 0.0
    error_count: int = 0


def run_cli_mode_bench(
    item: Input,
    rule_name: str,
    context: QgsProcessingContext,
    feedback: QgsProcessingFeedback,
) -> CLIBreakdown:
    """Measures CLI workflow broken down into distinct stages."""
    bd = CLIBreakdown()
    t_total_start = time.perf_counter()

    tmp_dir = Path(tempfile.mkdtemp(prefix='bench_cli_'))
    output_gpkg = str(tmp_dir / 'errors.gpkg')

    # 1. Feature iteration & coordinate inspection
    t_insp_start = time.perf_counter()
    req = QgsFeatureRequest().setSubsetOfAttributes([])
    for f in item.source.getFeatures(req):
        _ = invalid_reason(f.geometry())
    bd.feature_inspect_ms = (time.perf_counter() - t_insp_start) * 1000.0

    # 2. GeoPackage export to disk
    t_exp_start = time.perf_counter()
    prepared = export_dataset(item, tmp_dir, item.crs, context, feedback)
    bd.gpkg_export_ms = (time.perf_counter() - t_exp_start) * 1000.0

    # 3. CLI execution via subprocess
    rule_obj = Rule(rule_name, '', wkb_kind(item.source.wkbType()), None)
    check = Check(rule_obj, prepared.dataset)
    config_file = tmp_dir / 'config.toml'
    write_config(config_file, [check], tolerance=0.001)

    t_sub_start = time.perf_counter()
    completed = None
    try:
        completed = run_validate(
            CLI_EXECUTABLE,
            str(config_file),
            None,
            output_gpkg,
            feedback,
        )
    except Exception as e:
        # If run_validate raises RunError on non-zero exit code, retrieve completed if available
        pass
    bd.subprocess_exec_ms = (time.perf_counter() - t_sub_start) * 1000.0

    # 4. Result parsing & layer ingestion
    t_parse_start = time.perf_counter()
    if completed is not None:
        try:
            counts = rule_counts(completed)
            bd.error_count = sum(counts)
        except Exception:
            pass
    if Path(output_gpkg).exists():
        err_layer = QgsVectorLayer(output_gpkg, 'errors', 'ogr')
        if err_layer.isValid():
            bd.error_count = max(bd.error_count, err_layer.featureCount())
    bd.result_ingest_ms = (time.perf_counter() - t_parse_start) * 1000.0

    bd.total_ms = (time.perf_counter() - t_total_start) * 1000.0
    return bd


def run_native_mode_bench(
    item: Input,
    rule_name: str,
    context: QgsProcessingContext,
    feedback: QgsProcessingFeedback,
) -> NativeBreakdown:
    """Measures Native in-memory workflow broken down into distinct stages."""
    bd = NativeBreakdown()
    t_total_start = time.perf_counter()

    tmp_out = tempfile.mktemp(suffix='.gpkg')

    # 1. Feature iteration, WKB extraction & Shapely conversion in Python
    t_wkb_start = time.perf_counter()
    top = topology_checker.Topology(tolerance=0.001)
    req = QgsFeatureRequest().setSubsetOfAttributes([])
    fids = []
    wkbs = []
    for f in item.source.getFeatures(req):
        g = f.geometry()
        if not g.isNull() and not g.isEmpty():
            fids.append(f.id())
            wkbs.append(bytes(g.asWkb()))

    geoms = [shapely.from_wkb(wkb) for wkb in wkbs]
    l_id = top.add_layer(geoms, fids=fids, name=item.name)

    rule_enum = resolve_rule(rule_name)
    top.add_rule(rule_enum, l_id)
    bd.wkb_extract_ms = (time.perf_counter() - t_wkb_start) * 1000.0

    # 2. Rust validate
    t_rust_start = time.perf_counter()
    report = top.validate()
    bd.rust_validate_ms = (time.perf_counter() - t_rust_start) * 1000.0

    # 3. Result conversion
    t_conv_start = time.perf_counter()
    bd.error_count = sum(len(r.violations) for r in report.rules)
    for rule_rep in report.rules:
        for viol in rule_rep.violations:
            for sub in decompose_geom(viol.geometry):
                qg = QgsGeometry()
                qg.fromWkb(sub.wkb)
                qg.convertToMultiType()
    bd.result_convert_ms = (time.perf_counter() - t_conv_start) * 1000.0

    bd.total_ms = (time.perf_counter() - t_total_start) * 1000.0
    if Path(tmp_out).exists():
        Path(tmp_out).unlink()
    return bd


def run_raw_rust_bench(file_path: str, rule: str) -> float:
    """Runs raw topology-checker CLI directly on the file and measures wall-clock time."""
    cmd = [
        CLI_EXECUTABLE,
        *rule.split('/'),
        file_path,
        '--json',
    ]
    t0 = time.perf_counter()
    _ = subprocess.run(cmd, capture_output=True, text=True)
    t1 = time.perf_counter()
    return (t1 - t0) * 1000.0


def make_points_layer(n: int) -> QgsVectorLayer:
    layer = QgsVectorLayer('Point?crs=EPSG:4326', f'pts_{n}', 'memory')
    feats = []
    unique_count = int(n * 0.98)
    cols = int(math.isqrt(n))
    for i in range(n):
        idx = i if i < unique_count else (i % unique_count)
        x = idx % cols
        y = idx // cols
        f = QgsFeature()
        f.setGeometry(QgsGeometry.fromPointXY(QgsPointXY(float(x), float(y))))
        feats.append(f)
    layer.dataProvider().addFeatures(feats)
    layer.updateExtents()
    return layer


def make_polygons_layer(n: int) -> QgsVectorLayer:
    layer = QgsVectorLayer('Polygon?crs=EPSG:4326', f'poly_{n}', 'memory')
    feats = []
    cols = int(math.isqrt(n))
    for i in range(n):
        c = i % cols
        r = i // cols
        delta = 1.1 if (i % 50 == 0) else 0.9
        wkt = f'POLYGON(({c} {r}, {c+delta} {r}, {c+delta} {r+delta}, {c} {r+delta}, {c} {r}))'
        f = QgsFeature()
        f.setGeometry(QgsGeometry.fromWkt(wkt))
        feats.append(f)
    layer.dataProvider().addFeatures(feats)
    layer.updateExtents()
    return layer


def run_all():
    app = QApplication([])
    QgsApplication.initQgis()
    canvas = QgsMapCanvas()

    context = QgsProcessingContext()
    feedback = QgsProcessingFeedback()
    project = QgsProject.instance()

    all_results: list[dict[str, Any]] = []

    # -------------------------------------------------------------
    # 1. Real-World River Network Benchmark (CursuriApaRauri.shp)
    # -------------------------------------------------------------
    shp_path = '/tmp/cursuriaparauri/CursuriApaRauri.shp'
    if Path(shp_path).exists():
        print(
            '\n========================================================================'
        )
        print('=== 1. Real-World Rivers (4,917 LineStrings, 15.54M vertices, 237 MB) ===')
        print(
            '========================================================================'
        )
        rivers_layer = QgsVectorLayer(shp_path, 'rivers', 'ogr')
        project.addMapLayer(rivers_layer)
        item = Input('rivers', rivers_layer, rivers_layer)

        print('  [1/4] Running QGIS Built-in C++ Topology Checker...')
        cpp_ms = create_and_run_qgis_builtin_check(
            project, canvas, rivers_layer, 'must not have dangles'
        )
        print(f'        -> QGIS Built-in C++: {cpp_ms:,.2f} ms ({cpp_ms/1000:.2f} s)')

        print('  [2/4] Running Standalone Raw Rust CLI on Shapefile...')
        raw_rust_ms = run_raw_rust_bench(shp_path, 'line/must-not-have-dangles')
        print(f'        -> Raw Rust Engine: {raw_rust_ms:,.2f} ms ({raw_rust_ms/1000:.2f} s)')

        print('  [3/4] Running Plugin Native In-Memory Mode...')
        native_bd = run_native_mode_bench(
            item, 'line/must-not-have-dangles', context, feedback
        )
        print(f'        -> Plugin Native Total: {native_bd.total_ms:,.2f} ms ({native_bd.total_ms/1000:.2f} s)')
        print(f'           * WKB + Shapely extract: {native_bd.wkb_extract_ms:,.2f} ms')
        print(f'           * Rust validate:        {native_bd.rust_validate_ms:,.2f} ms')
        print(f'           * Result convert:       {native_bd.result_convert_ms:,.2f} ms')

        print('  [4/4] Running Plugin CLI Mode...')
        cli_bd = run_cli_mode_bench(
            item, 'line/must-not-have-dangles', context, feedback
        )
        print(f'        -> Plugin CLI Total: {cli_bd.total_ms:,.2f} ms ({cli_bd.total_ms/1000:.2f} s)')
        print(f'           * Feature inspect:  {cli_bd.feature_inspect_ms:,.2f} ms')
        print(f'           * GeoPackage write: {cli_bd.gpkg_export_ms:,.2f} ms')
        print(f'           * CLI subprocess:   {cli_bd.subprocess_exec_ms:,.2f} ms')
        print(f'           * Result ingest:    {cli_bd.result_ingest_ms:,.2f} ms')

        all_results.append({
            'dataset': 'Rivers Shapefile (15.5M vertices)',
            'features': 4917,
            'rule': 'line/must-not-have-dangles',
            'qgis_builtin_ms': cpp_ms,
            'raw_rust_ms': raw_rust_ms,
            'native': asdict(native_bd),
            'cli': asdict(cli_bd),
        })
        project.removeMapLayer(rivers_layer.id())

    # -------------------------------------------------------------
    # 2. Synthetic Datasets: Points (1k, 10k, 50k)
    # -------------------------------------------------------------
    for n in [1000, 10000, 50000]:
        print(f'\n=== 2. Synthetic Points ({n:,} features, point/must-not-overlap) ===')
        pt_layer = make_points_layer(n)
        project.addMapLayer(pt_layer)
        item = Input(f'pts_{n}', pt_layer, pt_layer)

        tmp_gpkg = tempfile.mktemp(suffix='.gpkg')
        options = QgsVectorFileWriter.SaveVectorOptions()
        options.driverName = 'GPKG'
        QgsVectorFileWriter.writeAsVectorFormatV3(
            pt_layer, tmp_gpkg, context.transformContext(), options
        )

        print('  [1/4] Running QGIS Built-in C++ Topology Checker...')
        cpp_ms = create_and_run_qgis_builtin_check(
            project, canvas, pt_layer, 'must not have duplicates'
        )
        print(f'        -> QGIS Built-in C++: {cpp_ms:,.2f} ms')

        print('  [2/4] Running Standalone Raw Rust CLI...')
        raw_rust_ms = run_raw_rust_bench(tmp_gpkg, 'point/must-not-overlap')
        print(f'        -> Raw Rust Engine: {raw_rust_ms:,.2f} ms')

        print('  [3/4] Running Plugin Native In-Memory Mode...')
        native_bd = run_native_mode_bench(
            item, 'point/must-not-overlap', context, feedback
        )
        print(f'        -> Plugin Native Total: {native_bd.total_ms:,.2f} ms')
        print(f'           * WKB + Shapely extract: {native_bd.wkb_extract_ms:,.2f} ms')
        print(f'           * Rust validate:        {native_bd.rust_validate_ms:,.2f} ms')
        print(f'           * Result convert:       {native_bd.result_convert_ms:,.2f} ms')

        print('  [4/4] Running Plugin CLI Mode...')
        cli_bd = run_cli_mode_bench(
            item, 'point/must-not-overlap', context, feedback
        )
        print(f'        -> Plugin CLI Total: {cli_bd.total_ms:,.2f} ms')
        print(f'           * Feature inspect:  {cli_bd.feature_inspect_ms:,.2f} ms')
        print(f'           * GeoPackage write: {cli_bd.gpkg_export_ms:,.2f} ms')
        print(f'           * CLI subprocess:   {cli_bd.subprocess_exec_ms:,.2f} ms')
        print(f'           * Result ingest:    {cli_bd.result_ingest_ms:,.2f} ms')

        all_results.append({
            'dataset': f'Points ({n:,})',
            'features': n,
            'rule': 'point/must-not-overlap',
            'qgis_builtin_ms': cpp_ms,
            'raw_rust_ms': raw_rust_ms,
            'native': asdict(native_bd),
            'cli': asdict(cli_bd),
        })

        if Path(tmp_gpkg).exists():
            Path(tmp_gpkg).unlink()
        project.removeMapLayer(pt_layer.id())

    # -------------------------------------------------------------
    # 3. Synthetic Datasets: Polygons (1k, 10k, 50k)
    # -------------------------------------------------------------
    for n in [1000, 10000, 50000]:
        print(f'\n=== 3. Synthetic Polygons ({n:,} features, polygon/must-not-overlap) ===')
        poly_layer = make_polygons_layer(n)
        project.addMapLayer(poly_layer)
        item = Input(f'poly_{n}', poly_layer, poly_layer)

        tmp_gpkg = tempfile.mktemp(suffix='.gpkg')
        options = QgsVectorFileWriter.SaveVectorOptions()
        options.driverName = 'GPKG'
        QgsVectorFileWriter.writeAsVectorFormatV3(
            poly_layer, tmp_gpkg, context.transformContext(), options
        )

        print('  [1/4] Running QGIS Built-in C++ Topology Checker...')
        cpp_ms = create_and_run_qgis_builtin_check(
            project, canvas, poly_layer, 'must not overlap'
        )
        print(f'        -> QGIS Built-in C++: {cpp_ms:,.2f} ms')

        print('  [2/4] Running Standalone Raw Rust CLI...')
        raw_rust_ms = run_raw_rust_bench(tmp_gpkg, 'polygon/must-not-overlap')
        print(f'        -> Raw Rust Engine: {raw_rust_ms:,.2f} ms')

        print('  [3/4] Running Plugin Native In-Memory Mode...')
        native_bd = run_native_mode_bench(
            item, 'polygon/must-not-overlap', context, feedback
        )
        print(f'        -> Plugin Native Total: {native_bd.total_ms:,.2f} ms')
        print(f'           * WKB + Shapely extract: {native_bd.wkb_extract_ms:,.2f} ms')
        print(f'           * Rust validate:        {native_bd.rust_validate_ms:,.2f} ms')
        print(f'           * Result convert:       {native_bd.result_convert_ms:,.2f} ms')

        print('  [4/4] Running Plugin CLI Mode...')
        cli_bd = run_cli_mode_bench(
            item, 'polygon/must-not-overlap', context, feedback
        )
        print(f'        -> Plugin CLI Total: {cli_bd.total_ms:,.2f} ms')
        print(f'           * Feature inspect:  {cli_bd.feature_inspect_ms:,.2f} ms')
        print(f'           * GeoPackage write: {cli_bd.gpkg_export_ms:,.2f} ms')
        print(f'           * CLI subprocess:   {cli_bd.subprocess_exec_ms:,.2f} ms')
        print(f'           * Result ingest:    {cli_bd.result_ingest_ms:,.2f} ms')

        all_results.append({
            'dataset': f'Polygons ({n:,})',
            'features': n,
            'rule': 'polygon/must-not-overlap',
            'qgis_builtin_ms': cpp_ms,
            'raw_rust_ms': raw_rust_ms,
            'native': asdict(native_bd),
            'cli': asdict(cli_bd),
        })

        if Path(tmp_gpkg).exists():
            Path(tmp_gpkg).unlink()
        project.removeMapLayer(poly_layer.id())

    out_json = PLUGIN_DIR / 'benchmarks' / 'results.json'
    out_json.write_text(json.dumps(all_results, indent=2))
    print(
        f'\n========================================================================'
    )
    print(f'Benchmark complete! Full results saved to: {out_json}')
    print(
        f'========================================================================\n'
    )


if __name__ == '__main__':
    run_all()
    os._exit(0)
