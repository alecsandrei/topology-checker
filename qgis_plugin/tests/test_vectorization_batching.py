"""Tests for vectorization and batching optimizations."""

from __future__ import annotations

import types
import typing as t
from pathlib import Path

import pytest
from qgis.core import (
    QgsCoordinateReferenceSystem,
    QgsFeature,
    QgsGeometry,
    QgsPointXY,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsProject,
    QgsVectorLayer,
)
from qgis.PyQt.QtGui import QStandardItemModel

import topology_rules.core.native_runner as nr
from topology_rules.core.binary import Rule as BinaryRule
from topology_rules.core.checks import Job
from topology_rules.core.inputs import export_dataset
from topology_rules.core.invalid import Invalid, append_invalid
from topology_rules.core.layers import Input
from topology_rules.core.style import Grouping
from topology_rules.panel.rows import Row, fast_field_text, read_rows
from topology_rules.panel.tree import ROLE, fill


def test_read_rows_field_caching():
    """Verifies pre-resolved field index lookups and fast_field_text."""
    field_spec = (
        'None?field=origin_layer:string'
        '&field=destination_layer:string'
        '&field=rule:string'
        '&field=error_type:string'
        '&field=origin_fid:integer'
        '&field=destination_fid:integer'
    )
    layer = QgsVectorLayer(field_spec, 'error_layer', 'memory')

    f1 = QgsFeature(layer.fields())
    f1.setAttributes(['roads', 'rivers', 'crosses', 'intersection', 101, 202])

    f2 = QgsFeature(layer.fields())
    f2.setAttributes([None, 'NULL', 'dangle', 'dangle', None, None])

    f3 = QgsFeature(layer.fields())
    f3.setAttributes(['NULL', None, '', 'gap', -7, None])

    layer.dataProvider().addFeatures([f1, f2, f3])

    names = {'roads': 'Roads Layer', 'rivers': 'Rivers Layer'}
    rows = read_rows([layer], names)

    assert len(rows) == 3

    # Row 1: Fully populated valid values
    r1 = rows[0]
    assert r1.rule_name == 'crosses'
    assert r1.error == 'intersection'
    assert r1.layer == 'Roads Layer'
    assert r1.other == 'Rivers Layer'
    assert r1.layer_alias == 'roads'
    assert r1.other_alias == 'rivers'
    assert r1.feature == 101
    assert r1.other_feature == 202
    assert r1.error_fid == 1

    # Row 2: None and 'NULL' string attributes
    r2 = rows[1]
    assert r2.rule_name == 'dangle'
    assert r2.error == 'dangle'
    assert r2.layer == ''
    assert r2.other == ''
    assert r2.layer_alias == ''
    assert r2.other_alias == ''
    assert r2.feature is None
    assert r2.other_feature is None
    assert r2.error_fid == 2

    # Row 3: Empty string, negative FID
    r3 = rows[2]
    assert r3.rule_name == ''
    assert r3.error == 'gap'
    assert r3.layer == ''
    assert r3.feature == -7
    assert r3.other_feature is None
    assert r3.error_fid == 3

    # Direct fast_field_text negative index test
    assert fast_field_text(f1, -1) == ''
    assert fast_field_text(f1, 0) == 'roads'
    assert fast_field_text(f2, 0) == ''
    assert fast_field_text(f2, 1) == ''

    # Layer with missing fields (e.g. missing origin_fid and destination_fid)
    partial_spec = (
        'None?field=origin_layer:string'
        '&field=destination_layer:string'
        '&field=rule:string'
        '&field=error_type:string'
    )
    partial_layer = QgsVectorLayer(partial_spec, 'partial', 'memory')
    f_part = QgsFeature(partial_layer.fields())
    f_part.setAttributes(['roads', 'rivers', 'crosses', 'intersection'])
    partial_layer.dataProvider().addFeatures([f_part])

    partial_rows = read_rows([partial_layer], names)
    assert len(partial_rows) == 1
    assert partial_rows[0].feature is None
    assert partial_rows[0].other_feature is None
    assert partial_rows[0].rule_name == 'crosses'

    # Layer with custom grouping and missing origin_layer
    minimal_spec = 'None?field=rule:string&field=error_type:string'
    minimal_layer = QgsVectorLayer(minimal_spec, 'minimal', 'memory')
    f_min = QgsFeature(minimal_layer.fields())
    f_min.setAttributes(['gap', 'gap'])
    minimal_layer.dataProvider().addFeatures([f_min])

    custom_grouping = Grouping('', lambda f: 'custom_group')
    min_rows = read_rows([minimal_layer], {}, grouping=custom_grouping)
    assert len(min_rows) == 1
    assert min_rows[0].layer == ''
    assert min_rows[0].other == ''
    assert min_rows[0].feature is None
    assert min_rows[0].key == 'custom_group'


def test_tree_fill_batched_items():
    """Verifies that batched appendRows in tree.fill constructs the tree."""
    model = QStandardItemModel()

    rows = [
        Row(
            'dangles',
            'dangle',
            'roads',
            1,
            '',
            None,
            'roads',
            '',
            'err1',
            1,
            key='rule1',
        ),
        Row(
            'dangles',
            'dangle',
            'roads',
            2,
            '',
            None,
            'roads',
            '',
            'err1',
            2,
            key='rule1',
        ),
        Row(
            'overlaps',
            'overlap',
            'parcels',
            10,
            'parcels',
            11,
            'parcels',
            'parcels',
            'err2',
            3,
            key='rule2',
        ),
        Row(
            'overlaps',
            'overlap',
            'parcels',
            12,
            'parcels',
            13,
            'parcels',
            'parcels',
            'err2',
            4,
            key='rule2',
        ),
    ]

    labels = {'rule1': 'Roads Dangles', 'rule2': 'Parcels Overlaps'}
    fill(model, rows, labels, ignored=lambda r: r.feature == 2)

    # 2 top-level rule groups
    assert model.rowCount() == 2

    # Group 1: rule1
    g1 = model.item(0)
    assert g1 is not None
    assert g1.text() == 'Roads Dangles (2)'
    assert g1.data(ROLE) == ('group', 'rule1', None)
    assert g1.rowCount() == 1  # 1 error kind (dangle)

    kind1 = g1.child(0)
    assert kind1 is not None
    assert kind1.text() == 'Dangles (2)'
    assert kind1.data(ROLE) == ('group', 'rule1', 'dangle')
    assert kind1.rowCount() == 2  # 2 leaf error rows

    leaf1 = kind1.child(0)
    assert leaf1 is not None
    assert leaf1.text() == 'roads 1'
    assert leaf1.data(ROLE) == rows[0]

    leaf2 = kind1.child(1)
    assert leaf2 is not None
    assert leaf2.text() == 'roads 2'
    assert leaf2.data(ROLE) == rows[1]
    # Ignored feature 2 has gray foreground
    assert leaf2.foreground().color().name() == '#a0a0a4'

    # Group 2: rule2
    g2 = model.item(1)
    assert g2 is not None
    assert g2.text() == 'Parcels Overlaps (2)'
    assert g2.data(ROLE) == ('group', 'rule2', None)


def test_export_dataset_chunking_over_batch_size(tmp_path: Path):
    """Verifies batched export with > 2,000 features writes every feature."""
    layer = QgsVectorLayer('Point?crs=EPSG:32633', 'bulk_points', 'memory')
    feature_count = 2100
    feats: list[QgsFeature] = []
    for i in range(1, feature_count + 1):
        f = QgsFeature(i)
        f.setGeometry(QgsGeometry.fromPointXY(QgsPointXY(float(i), float(i))))
        feats.append(f)

    layer.dataProvider().addFeatures(feats)
    assert layer.featureCount() == feature_count

    context = QgsProcessingContext()
    context.setProject(QgsProject.instance())
    feedback = QgsProcessingFeedback()
    inp = Input(name='bulk_points', source=layer, layer=layer)

    prep = export_dataset(inp, tmp_path, layer.crs(), context, feedback)

    # All IDs captured in order
    assert len(prep.ids) == feature_count
    assert prep.ids[0] == 1
    assert prep.ids[2000] == 2001
    assert prep.ids[-1] == feature_count

    # GeoPackage layer read back
    exported = QgsVectorLayer(
        f'{prep.dataset.path}|layername={prep.dataset.layer}', 'exported', 'ogr'
    )
    assert exported.isValid()
    assert exported.featureCount() == feature_count

    # Verify geometries and FIDs match
    exported_fids = [feat.id() for feat in exported.getFeatures()]
    assert len(exported_fids) == feature_count
    assert exported_fids == list(range(1, feature_count + 1))


def test_invalid_and_native_runner_batch_writes(tmp_path: Path):
    """Verifies batch writing of invalid features and native runner errors."""
    crs = QgsCoordinateReferenceSystem('EPSG:32633')
    context = QgsProcessingContext()
    gpkg_path = str(tmp_path / 'batch_errors.gpkg')

    # 1. Test append_invalid with 35 points and 25 lines
    invalid_items: list[Invalid] = []
    for i in range(35):
        pt_geom = QgsGeometry.fromPointXY(QgsPointXY(float(i), float(i)))
        invalid_items.append(
            Invalid('points_layer', i + 1, pt_geom, 'invalid coordinate')
        )
    for i in range(25):
        ln_geom = QgsGeometry.fromPolylineXY(
            [QgsPointXY(float(i), 0.0), QgsPointXY(float(i), 10.0)]
        )
        invalid_items.append(
            Invalid('lines_layer', i + 1, ln_geom, 'invalid line')
        )

    append_invalid(gpkg_path, invalid_items, crs, context)

    pt_err = QgsVectorLayer(f'{gpkg_path}|layername=errors_point', 'pt', 'ogr')
    ln_err = QgsVectorLayer(f'{gpkg_path}|layername=errors_line', 'ln', 'ogr')

    assert pt_err.isValid()
    assert pt_err.featureCount() == 35
    assert ln_err.isValid()
    assert ln_err.featureCount() == 25

    # 2. Test run_native_jobs batch write with multiple violations
    shapely = pytest.importorskip('shapely')

    class MockRef:
        def __init__(self, layer_idx: int, fid: int):
            self.layer = types.SimpleNamespace(index=layer_idx)
            self.fid = fid
            self.part = 0

    class MockViolation:
        def __init__(
            self,
            geom: t.Any,
            origin: t.Any,
            dest: t.Any,
            rule_val: str,
            err_val: str,
        ):
            self.geometry = geom
            self.origin = origin
            self.destination = dest
            self.resolved_rule = types.SimpleNamespace(value=rule_val)
            self.error_type = types.SimpleNamespace(value=err_val)

    class MockTopology:
        def __init__(self, tolerance: float = 0.0):
            self.layers: list[t.Any] = []
            self.rules: list[t.Any] = []

        def add_layer(
            self,
            data: t.Any,
            name: str | None = None,
            fids: t.Iterable[int] | None = None,
        ) -> t.Any:
            idx = len(self.layers)
            self.layers.append(types.SimpleNamespace(name=name, index=idx))
            return types.SimpleNamespace(index=idx)

        def add_rule(self, rule: t.Any, l1: t.Any, other: t.Any = None) -> None:
            self.rules.append((rule, l1, other))

        def validate(self) -> t.Any:
            v_list: list[t.Any] = []
            # 40 point violations
            for i in range(40):
                pt = shapely.Point(i, i)
                v_list.append(
                    MockViolation(
                        pt,
                        MockRef(0, i + 1),
                        None,
                        'point_rule',
                        'point_error',
                    )
                )
            # 20 line violations
            for i in range(20):
                line = shapely.LineString([(i, 0), (i, 10)])
                v_list.append(
                    MockViolation(
                        line,
                        MockRef(0, i + 1),
                        None,
                        'line_rule',
                        'line_error',
                    )
                )
            return types.SimpleNamespace(
                rules=[types.SimpleNamespace(violations=v_list)]
            )

    orig_top = nr.Topology
    orig_rule = nr.Rule
    orig_groups = nr._GROUPS
    try:
        nr.Topology = MockTopology  # type: ignore[assignment]
        nr.Rule = lambda x: types.SimpleNamespace(value=x)  # type: ignore[assignment]
        nr._GROUPS = {}

        poly_lyr = QgsVectorLayer('Point?crs=EPSG:32633', 'sample', 'memory')
        f = QgsFeature()
        f.setGeometry(QgsGeometry.fromPointXY(QgsPointXY(0.0, 0.0)))
        poly_lyr.dataProvider().addFeatures([f])

        input_item = Input(name='sample', source=poly_lyr, layer=poly_lyr)
        rule = BinaryRule(
            name='test/rule', description='', layer='point', other=None
        )
        job = Job(rule=rule, layer=input_item)

        native_out = str(tmp_path / 'native_batch_errors.gpkg')
        feedback = QgsProcessingFeedback()
        outcome = nr.run_native_jobs(
            [job], 0.001, native_out, context, feedback
        )

        assert outcome.counts == [60]
        n_pt_err = QgsVectorLayer(
            f'{native_out}|layername=errors_point', 'n_pt', 'ogr'
        )
        n_ln_err = QgsVectorLayer(
            f'{native_out}|layername=errors_line', 'n_ln', 'ogr'
        )

        assert n_pt_err.isValid()
        assert n_pt_err.featureCount() == 40
        assert n_ln_err.isValid()
        assert n_ln_err.featureCount() == 20
    finally:
        nr.Topology = orig_top
        nr.Rule = orig_rule
        nr._GROUPS = orig_groups
