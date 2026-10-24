"""The Python package and the executable find the same errors in the same
data, and write them to the same GeoPackage: the same layers, columns,
geometry types, CRS and rows, and the same warnings in the log."""

from __future__ import annotations

import collections.abc as c
import contextlib
import dataclasses
import sqlite3
import typing as t
from pathlib import Path

import pytest
from qgis.core import (
    QgsCoordinateReferenceSystem,
    QgsFeature,
    QgsGeometry,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsProject,
    QgsRectangle,
    QgsVectorLayer,
)

from topology_rules.core import checks, commands, engine, runner
from topology_rules.core.checks import Job, Outcome, run_jobs
from topology_rules.core.engine import Package
from topology_rules.core.layers import Input
from topology_rules.core.report import IdMap, Report, source_id
from topology_rules.exceptions import RunError

from .helpers import (
    ED50,
    ED50_FEET,
    ED50_FEET_NAVD88,
    line_layer,
    point_layer,
    polygon_layer,
    save,
)

# Two lines meeting at a node, and a line far from them: four dangles
CHAIN = (
    'LINESTRING(0 0, 10 0)',
    'LINESTRING(10 0, 20 0)',
    'LINESTRING(0 50, 10 50)',
)
SQUARE = 'POLYGON((0 0, 10 0, 10 10, 0 10, 0 0))'
# Overlaps the square
SHIFTED = 'POLYGON((5 5, 15 5, 15 15, 5 15, 5 5))'
BOWTIE = 'POLYGON((20 0, 30 10, 30 0, 20 10, 20 0))'
# Lines apart, two dangles each, whose ids are not in the order of their
# places, as an index has them
SCATTERED = tuple(
    f'LINESTRING({x} 0, {x} 5)' for x in (n * 17 % 40 * 10 for n in range(40))
)


@dataclasses.dataclass(frozen=True)
class Table:
    """A layer of errors as it is in the file."""

    columns: list[tuple[str, str, int]]
    geometry: tuple[str, str, int, int, str]
    rows: list[list[object]]


@dataclasses.dataclass(frozen=True)
class Written:
    """What a run gave: its outcome, the layers of errors that it wrote, and
    the warnings of its log."""

    outcome: Outcome
    tables: dict[str, Table] | None
    warnings: list[str]
    # What the executable writes itself from the same definition, with the
    # ids of the source features
    reference: dict[str, Table] | None = None


def wkt(blob: bytes) -> tuple[str, str]:
    """The geometry of a GeoPackage, as text, and as WKB to the last bit of
    each coordinate."""
    envelope = [0, 32, 48, 48, 64][(blob[3] >> 1) & 7]
    geometry = QgsGeometry()
    geometry.fromWkb(blob[8 + envelope :])
    return geometry.asWkt(6), bytes(geometry.asWkb()).hex()


def tables(path: Path, ids: IdMap) -> dict[str, Table]:
    """The layers of errors in the GeoPackage, with the ids of the source
    features, as the plugin shows them."""
    found: dict[str, Table] = {}
    with contextlib.closing(sqlite3.connect(path)) as db:
        layers = db.execute(
            'select g.table_name, g.column_name, g.geometry_type_name, g.z, '
            "g.m, s.organization || ':' || s.organization_coordsys_id "
            'from gpkg_geometry_columns g join gpkg_spatial_ref_sys s '
            'on s.srs_id = g.srs_id order by g.table_name'
        ).fetchall()
        for name, column, kind, z, m, srs in layers:
            # The executable declares the geometry column a BLOB, QGIS gives
            # it the geometry type, and readers take the type from
            # gpkg_geometry_columns
            columns = [
                (
                    str(info[1]),
                    kind if info[1] == column else str(info[2]),
                    int(info[5]),
                )
                for info in db.execute(f'pragma table_info("{name}")')
            ]
            names = [entry[0] for entry in columns]
            rows: list[list[object]] = []
            for values in db.execute(f'select * from "{name}" order by fid'):
                row = dict(zip(names, values, strict=True))
                row[column] = wkt(row[column])
                for layer, fid in (
                    ('origin_layer', 'origin_fid'),
                    ('destination_layer', 'destination_fid'),
                ):
                    row[fid] = source_id(ids, row[layer], row[fid])
                rows.append(list(row.values()))
            found[name] = Table(columns, (column, kind, z, m, srs), rows)
    return found


References = list[tuple[Path, IdMap]]


def recording(monkeypatch: pytest.MonkeyPatch, references: References) -> None:
    """Has the executable also write the errors of each run of the plugin
    itself, to a GeoPackage next to the definition, with the ids of the
    source features of the run."""
    made: list[IdMap] = []

    class Recorded(Report):
        def __init__(self, gathered: t.Any, ids: IdMap) -> None:
            super().__init__(gathered, ids)
            made.append(ids)

    def run_validate(
        executable: str,
        config: str,
        tolerance: float | None,
        *args: t.Any,
        **kwargs: t.Any,
    ) -> runner.Completed:
        completed = commands.run_validate(
            executable, config, tolerance, *args, **kwargs
        )
        path = Path(config).with_name('reference.gpkg')
        command = commands.definition_command(
            executable, config, tolerance, str(path)
        )
        if runner.run(command).exit_code in (0, 1):
            references.append((path, made[-1]))
        return completed

    monkeypatch.setattr(checks, 'Report', Recorded)
    monkeypatch.setattr(checks, 'run_validate', run_validate)


def run(
    tool: engine.Tool,
    jobs: list[Job],
    folder: Path,
    tolerance: float | None,
    referenced: bool = False,
) -> Written:
    """What a run of the jobs gave. With the executable, if referenced, also
    what the executable writes itself from the same definition."""
    folder.mkdir()
    work = folder / 'work'
    work.mkdir()
    output = folder / 'errors.gpkg'
    context = QgsProcessingContext()
    context.setProject(QgsProject.instance())
    feedback = QgsProcessingFeedback()
    references: References = []
    with pytest.MonkeyPatch.context() as patch:
        if referenced and isinstance(tool, str):
            recording(patch, references)
        outcome = run_jobs(
            jobs, tool, tolerance, str(output), work, context, feedback
        )
    warnings = sorted(
        {
            line
            for line in feedback.textLog().splitlines()
            if line.startswith('warning: ')
        }
    )
    reference = None
    if references:
        path, ids = references[-1]
        reference = tables(path, ids) if path.exists() else None
    return Written(
        outcome,
        tables(output, {}) if output.exists() else None,
        warnings,
        reference,
    )


def same_tables(
    first: dict[str, Table] | None,
    second: dict[str, Table] | None,
    same_srs: bool = True,
) -> None:
    """Compares the layers one part at a time, which shows what differs.
    Without the same SRS, they need only have one of the same organization."""
    if first is None or second is None:
        assert first == second
        return
    assert list(first) == list(second)
    for name, table in first.items():
        other = second[name]
        assert table.columns == other.columns, name
        if same_srs:
            assert table.geometry == other.geometry, name
        else:
            *kind, srs = table.geometry
            *other_kind, other_srs = other.geometry
            assert kind == other_kind, name
            assert srs.split(':')[0] == other_srs.split(':')[0], name
        assert table.rows == other.rows, name


Agree = c.Callable[..., Written]


@pytest.fixture
def agree(executable: str, package: Package, tmp_path: Path) -> Agree:
    """Runs jobs with both engines, checks that they agree with each other
    and with what the executable writes itself, and returns what the package
    did. Without the same SRS, the executable's own file need only have a
    CRS of the same organization."""
    rules = {rule.name: rule for rule in engine.rules(package)}
    runs = iter(range(1000))

    def check(
        *jobs: tuple[str, Input] | tuple[str, Input, Input],
        tolerance: float | None = None,
        same_srs: bool = True,
    ) -> Written:
        made = [Job(rules[name], *inputs) for name, *inputs in jobs]
        number = next(runs)
        by_executable = run(
            executable,
            made,
            tmp_path / f'executable{number}',
            tolerance,
            referenced=True,
        )
        by_package = run(
            package, made, tmp_path / f'package{number}', tolerance
        )
        same_tables(by_executable.reference, by_executable.tables, same_srs)
        same_tables(by_package.tables, by_executable.tables)
        assert by_package.warnings == by_executable.warnings
        first, second = by_package.outcome, by_executable.outcome
        assert first.counts == second.counts
        assert first.layers == second.layers
        assert [(i.alias, i.fid, i.reason) for i in first.invalid] == [
            (i.alias, i.fid, i.reason) for i in second.invalid
        ]
        assert first.crs == second.crs
        return by_package

    return check


def origin_ids(written: Written) -> list[object]:
    """The ids of the features of the point errors, in the order of the
    file."""
    assert written.tables is not None
    table = written.tables['errors_point']
    index = [entry[0] for entry in table.columns].index('origin_fid')
    return [row[index] for row in table.rows]


def column(written: Written, name: str) -> set[object]:
    """The values of a column in the layers of errors."""
    assert written.tables is not None
    return {
        row[[entry[0] for entry in table.columns].index(name)]
        for table in written.tables.values()
        for row in table.rows
    }


def project(layer: QgsVectorLayer) -> Input:
    """The whole layer, added to the project."""
    QgsProject.instance().addMapLayer(layer)
    return Input(layer.name(), layer, layer)


def saved(
    directory: Path, layer: QgsVectorLayer, name: str | None = None
) -> QgsVectorLayer:
    """The layer in a GeoPackage of its own, which the executable reads."""
    name = name or layer.name()
    path = directory / f'{name}.gpkg'
    save(layer, path, 'GPKG', name)
    found = QgsVectorLayer(f'{path}|layername={name}', name, 'ogr')
    assert found.isValid()
    return found


def memory(
    kind: str, crs: str, *wkts: str | None, name: str = 'layer'
) -> QgsVectorLayer:
    """A memory layer, whose features without a text have no geometry."""
    layer = QgsVectorLayer(f'{kind}?crs={crs}' if crs else kind, name, 'memory')
    features: list[QgsFeature] = []
    for text in wkts:
        feature = QgsFeature()
        if text is not None:
            feature.setGeometry(QgsGeometry.fromWkt(text))
        features.append(feature)
    layer.dataProvider().addFeatures(features)
    layer.updateExtents()
    return layer


def test_the_rules_are_the_same(executable: str, package: Package):
    """The same names, descriptions and layers, so that a rule of the panel
    or the toolbox is the same whichever engine runs it."""
    by_name = {rule.name: rule for rule in engine.rules(executable)}
    assert {rule.name: rule for rule in engine.rules(package)} == by_name


def test_errors_of_a_memory_layer(agree: Agree):
    written = agree(('line/must-not-have-dangles', project(line_layer(*CHAIN))))
    assert written.outcome.counts == [4]
    assert written.tables is not None
    assert list(written.tables) == ['errors_point']


def test_no_errors_write_no_file(agree: Agree):
    ring = line_layer('LINESTRING(0 0, 10 0, 10 10, 0 0)')
    written = agree(('line/must-not-have-dangles', project(ring)))
    assert written.outcome.counts == [0]
    assert written.tables is None


def test_errors_of_several_rules_and_families(agree: Agree):
    lines = project(line_layer(*CHAIN, 'LINESTRING(0 -5, 5 5)'))
    polygons = project(polygon_layer(SQUARE, SHIFTED, name='parcels'))
    written = agree(
        ('line/must-not-have-dangles', lines),
        ('line/must-not-intersect', lines),
        ('polygon/must-not-overlap', polygons),
    )
    assert written.tables is not None
    assert len(written.tables) >= 2


def test_layers_of_files_are_named_after_them(agree: Agree, tmp_path: Path):
    roads = saved(tmp_path, line_layer(*CHAIN, name='roads'))
    again = QgsVectorLayer(roads.source(), 'roads again', 'ogr')
    rivers = saved(tmp_path, line_layer('LINESTRING(5 -5, 5 5)'), 'rivers')
    written = agree(
        ('line/must-not-have-dangles', project(roads)),
        ('line/must-not-intersect-with', project(again), project(rivers)),
    )
    assert column(written, 'origin_layer') == {'roads'}
    assert column(written, 'destination_layer') == {None, 'rivers'}


def test_two_layers_with_the_same_name(agree: Agree):
    first = project(line_layer(*CHAIN, name='lines'))
    second = project(line_layer('LINESTRING(5 -5, 5 5)', name='lines'))
    written = agree(('line/must-not-intersect-with', first, second))
    assert set(written.outcome.layers) == {'lines', 'lines_2'}


def test_an_input_in_another_crs_is_reprojected(agree: Agree):
    points = project(point_layer('POINT(500000 0)', 'POINT(600000 0)'))
    zone = memory(
        'Polygon',
        'EPSG:4326',
        'POLYGON((14.9 -0.1, 15.1 -0.1, 15.1 0.1, 14.9 0.1, 14.9 -0.1))',
        name='zone',
    )
    written = agree(('point/must-be-inside', points, project(zone)))
    assert written.outcome.counts == [1]


def test_degrees(agree: Agree):
    lines = memory(
        'LineString', 'EPSG:4326', 'LINESTRING(15 0, 15.1 0)', name='lines'
    )
    written = agree(('line/must-not-have-dangles', project(lines)))
    assert any('angular' in warning for warning in written.warnings)


@pytest.mark.parametrize(
    ('crs', 'gap'),
    [('EPSG:7415', 0.0004), ('EPSG:8759', 0.003), ('EPSG:9518', 5e-9)],
    ids=['metres', 'us feet', 'degrees'],
)
def test_a_compound_crs(agree: Agree, crs: str, gap: float):
    """The default tolerance is that of the horizontal part, which closes a
    gap below a millimetre, or below 1e-8 in degrees."""
    lines = memory(
        'LineString',
        crs,
        'LINESTRING(100 10, 100.5 10)',
        f'LINESTRING({100.5 + gap} 10, 101 10)',
        name='lines',
    )
    written = agree(('line/must-not-have-dangles', project(lines)))
    assert written.outcome.counts == [2]


@pytest.mark.parametrize(
    ('crs', 'gap'),
    [
        (f'WKT:{ED50}', 5e-9),
        (
            'PROJ:+proj=longlat +ellps=intl '
            '+towgs84=-87,-98,-121,0,0,0,0 +no_defs',
            5e-9,
        ),
        ('PROJ:+proj=longlat +ellps=bessel +no_defs', 5e-9),
        (f'WKT:{ED50_FEET}', 0.003),
        (f'WKT:{ED50_FEET_NAVD88}', 0.003),
    ],
    ids=[
        'degrees with a datum shift',
        'proj string with a datum shift',
        'degrees of no authority',
        'us feet with a datum shift',
        'compound with a datum shift',
    ],
)
def test_a_crs_outside_the_database(agree: Agree, crs: str, gap: float):
    """A CRS bound to WGS 84 by a datum shift, or one that QGIS does not find
    in its database, has the default tolerance of its horizontal unit."""
    lines = memory(
        'LineString',
        crs,
        'LINESTRING(100 10, 100.5 10)',
        f'LINESTRING({100.5 + gap} 10, 101 10)',
        name='lines',
    )
    assert lines.crs().isValid()
    # The executable writes a CRS of no authority without its name or datum
    # shift, as NONE:100001 where QGIS writes NONE:100000
    written = agree(
        ('line/must-not-have-dangles', project(lines)), same_srs=False
    )
    assert written.outcome.counts == [2]


def test_degrees_with_a_datum_shift(agree: Agree):
    lines = memory(
        'LineString', f'WKT:{ED50}', 'LINESTRING(15 0, 15.1 0)', name='lines'
    )
    written = agree(
        ('line/must-not-have-dangles', project(lines)), same_srs=False
    )
    assert any('angular' in warning for warning in written.warnings)


def test_a_layer_without_a_crs(agree: Agree):
    lines = memory('LineString', '', 'LINESTRING(0 0, 10 0)', name='lines')
    QgsProject.instance().addMapLayer(lines)
    # QGIS gives a layer without a CRS one as the layer is added
    lines.setCrs(QgsCoordinateReferenceSystem())
    written = agree(
        ('line/must-not-have-dangles', Input(lines.name(), lines, lines))
    )
    assert written.tables is not None
    assert {table.geometry[4] for table in written.tables.values()} == {
        'NONE:0'
    }


def test_empty_features_and_features_with_z(agree: Agree):
    lines = memory(
        'LineStringZ',
        'EPSG:32633',
        'LINESTRINGZ(0 0 1, 10 0 2)',
        'LINESTRINGZ EMPTY',
        name='lines',
    )
    written = agree(('line/must-not-have-dangles', project(lines)))
    assert written.warnings


@pytest.mark.parametrize('stored', [False, True], ids=['memory', 'file'])
def test_features_without_a_geometry(
    agree: Agree, tmp_path: Path, stored: bool
):
    """The log counts them, and empty ones, as the executable does. QGIS
    exports a missing geometry as an empty one, and a file can hold both."""
    lines = memory(
        'LineString',
        'EPSG:32633',
        'LINESTRING(0 0, 10 0)',
        None,
        'LINESTRING EMPTY',
        None,
        name='lines',
    )
    if stored:
        lines = saved(tmp_path, lines)
        missing = {fid: QgsGeometry() for fid in (2, 4)}
        assert lines.dataProvider().changeGeometryValues(missing)
    written = agree(('line/must-not-have-dangles', project(lines)))
    assert written.outcome.counts == [2]
    assert written.warnings == (
        [
            'warning: layer lines: skipped 1 empty geometries',
            'warning: layer lines: skipped 2 null geometries',
        ]
        if stored
        else ['warning: layer lines: skipped 3 empty geometries']
    )


def test_degenerate_parts(agree: Agree):
    """Lines whose points are the same, and rings of fewer than three
    points, which both leave out and warn about."""
    lines = memory(
        'MultiLineString',
        'EPSG:32633',
        'MULTILINESTRING((0 0, 10 0))',
        'MULTILINESTRING((50 50, 50 50))',
        'MULTILINESTRING((20 20, 30 20),(40 40, 40 40))',
        name='lines',
    )
    polygons = memory(
        'Polygon',
        'EPSG:32633',
        SQUARE,
        'POLYGON((0 0, 10 0, 10 10, 0 10, 0 0),(5 5, 6 5, 5 5))',
        'POLYGON((50 50, 60 50, 50 50))',
        name='polygons',
    )
    written = agree(
        ('line/must-not-have-dangles', project(lines)),
        ('polygon/must-not-overlap', project(polygons)),
    )
    assert any('degenerate' in warning for warning in written.warnings)


def test_curves(agree: Agree):
    lines = memory(
        'CompoundCurve',
        'EPSG:32633',
        'COMPOUNDCURVE(CIRCULARSTRING(0 0, 5 5, 10 0))',
        'COMPOUNDCURVE((10 0, 20 0))',
        name='curves',
    )
    agree(('line/must-not-have-dangles', project(lines)))


def test_invalid_polygons_of_a_memory_layer(agree: Agree):
    polygons = project(polygon_layer(SQUARE, SHIFTED, BOWTIE))
    written = agree(('polygon/must-not-overlap', polygons))
    assert [item.fid for item in written.outcome.invalid] == [3]


def test_invalid_polygons_of_a_file(agree: Agree, tmp_path: Path):
    polygons = saved(tmp_path, polygon_layer(SQUARE, SHIFTED, BOWTIE))
    written = agree(('polygon/must-not-overlap', project(polygons)))
    assert [item.fid for item in written.outcome.invalid] == [3]


def test_selected_features(agree: Agree):
    layer = line_layer(*CHAIN)
    QgsProject.instance().addMapLayer(layer)
    chosen = frozenset(feature.id() for feature in layer.getFeatures()) - {3}
    written = agree(
        (
            'line/must-not-have-dangles',
            Input(layer.name(), layer, layer, False, chosen),
        )
    )
    assert written.outcome.counts == [2]


def test_features_in_an_area(agree: Agree):
    layer = line_layer(*CHAIN)
    QgsProject.instance().addMapLayer(layer)
    area = QgsRectangle(-1, -1, 30, 10)
    written = agree(
        (
            'line/must-not-have-dangles',
            Input(layer.name(), layer, layer, False, area=area),
        )
    )
    assert written.outcome.counts == [2]


def test_many_selected_features(agree: Agree):
    """QGIS gives a selection in no set order, and the errors come in the
    order of the ids."""
    layer = line_layer(*SCATTERED)
    QgsProject.instance().addMapLayer(layer)
    chosen = frozenset(range(5, 35))
    written = agree(
        (
            'line/must-not-have-dangles',
            Input(layer.name(), layer, layer, False, chosen),
        )
    )
    assert origin_ids(written) == sorted(2 * list(chosen))


def test_features_in_an_area_of_a_layer_with_an_index(agree: Agree):
    """QGIS gives the features of an area in the order of its index."""
    layer = line_layer(*SCATTERED)
    assert layer.dataProvider().createSpatialIndex()
    QgsProject.instance().addMapLayer(layer)
    area = QgsRectangle(-1, -1, 300, 6)
    written = agree(
        (
            'line/must-not-have-dangles',
            Input(layer.name(), layer, layer, False, area=area),
        )
    )
    found = origin_ids(written)
    assert found == sorted(found)
    assert len(found) == 2 * 31


def test_new_features_of_an_edit_session(agree: Agree):
    """QGIS gives the new features first, the last one first."""
    layer = line_layer(*CHAIN)
    QgsProject.instance().addMapLayer(layer)
    assert layer.startEditing()
    try:
        for y in (100, 200, 300):
            feature = QgsFeature(layer.fields())
            feature.setGeometry(
                QgsGeometry.fromWkt(f'LINESTRING(0 {y}, 10 {y})')
            )
            assert layer.addFeature(feature)
        written = agree(
            (
                'line/must-not-have-dangles',
                Input(layer.name(), layer, layer),
            )
        )
    finally:
        layer.rollBack()
    found = origin_ids(written)
    assert found == sorted(found)
    assert len([fid for fid in found if t.cast(int, fid) < 0]) == 6


def test_a_tolerance(agree: Agree):
    lines = project(
        line_layer('LINESTRING(0 0, 10 0)', 'LINESTRING(10.5 0, 20 0)')
    )
    assert agree(
        ('line/must-not-have-dangles', lines), tolerance=1.0
    ).outcome.counts == [2]


def test_a_rule_that_does_not_fit_fails_alike(
    executable: str, package: Package, tmp_path: Path
):
    rules = {rule.name: rule for rule in engine.rules(package)}
    points = project(point_layer('POINT(0 0)'))
    job = Job(rules['line/must-not-have-dangles'], points)
    messages = []
    for number, tool in enumerate((executable, package)):
        with pytest.raises(RunError) as raised:
            run(tool, [job], tmp_path / str(number), None)
        messages.append(str(raised.value))
    assert all(messages)
