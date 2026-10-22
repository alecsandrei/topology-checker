"""Tests for the native in-memory validation engine and fallback wiring."""

from __future__ import annotations

import types
import typing as t
from enum import Enum
from pathlib import Path

import pytest
from qgis.core import (
    QgsCoordinateReferenceSystem,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsProject,
    QgsVectorLayer,
)

import topology_rules.core.native_runner as nr
from topology_rules.core.binary import Rule as BinaryRule
from topology_rules.core.checks import Job, Outcome, run_jobs
from topology_rules.core.layers import Input
from topology_rules.core.native_runner import (
    decompose_geom,
    resolve_rule,
    run_native_jobs,
)
from topology_rules.exceptions import RunError

from .helpers import line_layer, point_layer, polygon_layer

DANGLES = 'line/must-not-have-dangles'
COVERED_BY = 'point/must-be-covered-by-boundary-of'
CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
RING = ('LINESTRING(0 0, 10 0, 10 10, 0 0)',)


class MockRule(str, Enum):
    MUST_NOT_HAVE_DANGLES = 'must-not-have-dangles'
    MUST_CONTAIN_ONE = 'must-contain-one'
    MUST_BE_COVERED_BY_BOUNDARY = 'must-be-covered-by-boundary-of'


@pytest.fixture
def native_engine(monkeypatch: pytest.MonkeyPatch):
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

    class MockRuleReport:
        def __init__(self, violations: list[t.Any]):
            self.violations = tuple(violations)

    class MockReport:
        def __init__(self, rules: list[t.Any]):
            self.rules = tuple(rules)

    class MockTopology:
        def __init__(self, tolerance: float = 0.0):
            self.tolerance = tolerance
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

        def validate(self) -> MockReport:
            v_list = []
            for r, _l1, _other in self.rules:
                val = str(getattr(r, 'value', r)).lower()
                if 'dangle' in val:
                    pt1 = shapely.Point(0, 0)
                    pt2 = shapely.Point(20, 0)
                    v_list.append(
                        MockViolation(
                            pt1,
                            MockRef(0, 1),
                            None,
                            'line/must-not-have-dangles',
                            'dangle',
                        )
                    )
                    v_list.append(
                        MockViolation(
                            pt2,
                            MockRef(0, 2),
                            None,
                            'line/must-not-have-dangles',
                            'dangle',
                        )
                    )
                elif 'covered' in val:
                    pt = shapely.Point(5, 5)
                    v_list.append(
                        MockViolation(
                            pt,
                            MockRef(0, 42),
                            MockRef(1, 99),
                            'point/must-be-covered-by-boundary-of',
                            'not_covered',
                        )
                    )
            return MockReport([MockRuleReport(v_list)])

    monkeypatch.setattr(nr, 'Topology', MockTopology)
    monkeypatch.setattr(nr, 'Rule', MockRule)
    monkeypatch.setattr(
        nr,
        '_GROUPS',
        {
            MockRule.MUST_NOT_HAVE_DANGLES: ('line/must-not-have-dangles',),
            MockRule.MUST_CONTAIN_ONE: ('polygon/must-contain-one-point',),
            MockRule.MUST_BE_COVERED_BY_BOUNDARY: (
                'point/must-be-covered-by-boundary-of',
            ),
        },
    )


def test_resolve_rule_variations(native_engine: None):
    r1 = resolve_rule('line/must-not-have-dangles')
    assert r1.value == 'must-not-have-dangles'

    r2 = resolve_rule('must-not-have-dangles')
    assert r2.value == 'must-not-have-dangles'

    r3 = resolve_rule('polygon/must-contain-one-point')
    assert r3.value == 'must-contain-one'

    with pytest.raises(ValueError, match='Unknown rule'):
        resolve_rule('non-existent-rule-name-xyz')


def test_decompose_geom():
    shapely = pytest.importorskip('shapely')

    pt = shapely.Point(1, 1)
    assert decompose_geom(pt) == [pt]

    poly = shapely.Polygon([(0, 0), (1, 0), (1, 1), (0, 1), (0, 0)])
    assert decompose_geom(poly) == [poly]

    gc = shapely.GeometryCollection([pt, poly])
    decomposed = decompose_geom(gc)
    assert decomposed == [pt, poly]


def test_run_native_jobs_empty(tmp_path: Path, native_engine: None):
    context = QgsProcessingContext()
    feedback = QgsProcessingFeedback()
    output = str(tmp_path / 'errors.gpkg')

    outcome = run_native_jobs([], 0.001, output, context, feedback)
    assert outcome.counts == []
    assert outcome.ids == {}
    assert outcome.layers == {}


def test_run_native_jobs_detects_violations(
    tmp_path: Path, native_engine: None
):
    roads = line_layer(*CHAIN, name='roads')
    QgsProject.instance().addMapLayer(roads)
    inp = Input(name='roads', source=roads, layer=roads)
    rule = BinaryRule(
        name=DANGLES,
        description='',
        layer='line',
        other=None,
    )
    job = Job(rule=rule, layer=inp)

    context = QgsProcessingContext()
    context.setProject(QgsProject.instance())
    feedback = QgsProcessingFeedback()
    output = str(tmp_path / 'errors.gpkg')

    outcome = run_native_jobs([job], 0.001, output, context, feedback)

    assert outcome.counts == [2]
    assert outcome.ids == {}
    assert outcome.layers == {'roads': roads.id()}
    assert Path(output).exists()

    err_layer = QgsVectorLayer(f'{output}|layername=errors_point', 'err', 'ogr')
    assert err_layer.isValid()
    assert err_layer.featureCount() == 2

    features = list(err_layer.getFeatures())
    for feat in features:
        assert feat['rule'] == DANGLES
        assert feat['error_type'] == 'dangle'
        assert feat['origin_layer'] == 'roads'
        assert feat['origin_fid'] in [1, 2]


def test_run_native_jobs_clean_run(
    tmp_path: Path, native_engine: None, monkeypatch: pytest.MonkeyPatch
):
    roads = line_layer(*RING, name='roads_ring')
    QgsProject.instance().addMapLayer(roads)
    inp = Input(name='roads_ring', source=roads, layer=roads)
    rule = BinaryRule(
        name=DANGLES,
        description='',
        layer='line',
        other=None,
    )
    job = Job(rule=rule, layer=inp)

    class CleanTopology(nr.Topology):  # type: ignore[name-defined]
        def validate(self) -> t.Any:
            return types.SimpleNamespace(
                rules=[types.SimpleNamespace(violations=())]
            )

    monkeypatch.setattr(nr, 'Topology', CleanTopology)

    context = QgsProcessingContext()
    context.setProject(QgsProject.instance())
    feedback = QgsProcessingFeedback()
    output = str(tmp_path / 'errors.gpkg')

    outcome = run_native_jobs([job], 0.001, output, context, feedback)

    assert outcome.counts == [0]
    assert outcome.ids == {}
    assert Path(output).exists()


def test_run_native_jobs_cancellation(tmp_path: Path, native_engine: None):
    roads = line_layer(*CHAIN, name='roads')
    inp = Input(name='roads', source=roads, layer=roads)
    rule = BinaryRule(
        name=DANGLES,
        description='',
        layer='line',
        other=None,
    )
    job = Job(rule=rule, layer=inp)

    context = QgsProcessingContext()
    feedback = QgsProcessingFeedback()
    feedback.cancel()
    output = str(tmp_path / 'errors.gpkg')

    with pytest.raises(RunError, match='Canceled'):
        run_native_jobs([job], 0.001, output, context, feedback)


def test_run_jobs_dispatches_to_native(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    roads = line_layer(*CHAIN, name='roads')
    inp = Input(name='roads', source=roads, layer=roads)
    rule = BinaryRule(
        name=DANGLES,
        description='',
        layer='line',
        other=None,
    )
    job = Job(rule=rule, layer=inp)

    expected = Outcome(
        counts=[2],
        ids={},
        layers={'roads': 'id1'},
        invalid=[],
        crs=QgsCoordinateReferenceSystem('EPSG:4326'),
    )

    monkeypatch.setattr(
        'topology_rules.core.checks.native_available', lambda: True
    )
    monkeypatch.setattr(
        'topology_rules.core.checks.run_native_jobs', lambda *a, **k: expected
    )

    context = QgsProcessingContext()
    feedback = QgsProcessingFeedback()
    res = run_jobs(
        [job],
        'fake-cli',
        0.001,
        str(tmp_path / 'errors.gpkg'),
        tmp_path,
        context,
        feedback,
    )
    assert res is expected


def test_run_jobs_fallback_on_native_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    roads = line_layer(*CHAIN, name='roads')
    inp = Input(name='roads', source=roads, layer=roads)
    rule = BinaryRule(
        name=DANGLES,
        description='',
        layer='line',
        other=None,
    )
    job = Job(rule=rule, layer=inp)

    def failing_native(*a: t.Any, **k: t.Any) -> Outcome:
        raise RuntimeError('Simulated in-memory failure')

    cli_called = False

    def fake_run_once(*a: t.Any, **k: t.Any) -> Outcome:
        nonlocal cli_called
        cli_called = True
        return Outcome(
            counts=[2],
            ids={},
            layers={},
            invalid=[],
            crs=QgsCoordinateReferenceSystem(),
        )

    monkeypatch.setattr(
        'topology_rules.core.checks.native_available', lambda: True
    )
    monkeypatch.setattr(
        'topology_rules.core.checks.run_native_jobs', failing_native
    )
    monkeypatch.setattr('topology_rules.core.checks.run_once', fake_run_once)

    info_messages: list[str] = []
    feedback = QgsProcessingFeedback()
    feedback.pushInfo = lambda msg: info_messages.append(msg)

    context = QgsProcessingContext()
    run_jobs(
        [job],
        'fake-cli',
        0.001,
        str(tmp_path / 'errors.gpkg'),
        tmp_path,
        context,
        feedback,
    )

    assert cli_called is True
    assert any(
        'Native engine failed' in msg and 'falling back to CLI' in msg
        for msg in info_messages
    )


def test_run_jobs_uses_cli_when_native_unavailable(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    roads = line_layer(*CHAIN, name='roads')
    inp = Input(name='roads', source=roads, layer=roads)
    rule = BinaryRule(
        name=DANGLES,
        description='',
        layer='line',
        other=None,
    )
    job = Job(rule=rule, layer=inp)

    native_called = False

    def fake_native(*a: t.Any, **k: t.Any) -> Outcome:
        nonlocal native_called
        native_called = True
        return Outcome(
            counts=[],
            ids={},
            layers={},
            invalid=[],
            crs=QgsCoordinateReferenceSystem(),
        )

    cli_called = False

    def fake_run_once(*a: t.Any, **k: t.Any) -> Outcome:
        nonlocal cli_called
        cli_called = True
        return Outcome(
            counts=[1],
            ids={},
            layers={},
            invalid=[],
            crs=QgsCoordinateReferenceSystem(),
        )

    monkeypatch.setattr(
        'topology_rules.core.checks.native_available', lambda: False
    )
    monkeypatch.setattr(
        'topology_rules.core.checks.run_native_jobs', fake_native
    )
    monkeypatch.setattr('topology_rules.core.checks.run_once', fake_run_once)

    context = QgsProcessingContext()
    feedback = QgsProcessingFeedback()
    res = run_jobs(
        [job],
        'fake-cli',
        0.001,
        str(tmp_path / 'errors.gpkg'),
        tmp_path,
        context,
        feedback,
    )

    assert native_called is False
    assert cli_called is True
    assert res.counts == [1]


def test_run_native_jobs_two_layers_destination_fid(
    tmp_path: Path, native_engine: None
):
    pts = point_layer('POINT(5 5)', name='sample_points')
    polys = polygon_layer(
        'POLYGON((0 0, 10 0, 10 10, 0 10, 0 0))', name='sample_polys'
    )
    QgsProject.instance().addMapLayer(pts)
    QgsProject.instance().addMapLayer(polys)

    inp1 = Input(name='sample_points', source=pts, layer=pts)
    inp2 = Input(name='sample_polys', source=polys, layer=polys)

    rule = BinaryRule(
        name=COVERED_BY,
        description='',
        layer='point',
        other='polygon',
    )
    job = Job(rule=rule, layer=inp1, other=inp2)

    context = QgsProcessingContext()
    context.setProject(QgsProject.instance())
    feedback = QgsProcessingFeedback()
    output = str(tmp_path / 'errors.gpkg')

    outcome = run_native_jobs([job], 0.001, output, context, feedback)

    assert outcome.counts == [1]
    assert outcome.layers == {
        'sample_points': pts.id(),
        'sample_polys': polys.id(),
    }

    err_layer = QgsVectorLayer(f'{output}|layername=errors_point', 'err', 'ogr')
    assert err_layer.isValid()
    assert err_layer.featureCount() == 1

    feat = next(err_layer.getFeatures())
    assert feat['rule'] == COVERED_BY
    assert feat['error_type'] == 'not_covered'
    assert feat['origin_layer'] == 'sample_points'
    assert feat['origin_fid'] == 42
    assert feat['destination_layer'] == 'sample_polys'
    assert feat['destination_fid'] == 99
