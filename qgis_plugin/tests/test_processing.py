"""The ways of driving the algorithms that Processing offers beside the
dialog: models, loops and concurrent tasks, the `qgis_process` program, and
the Python command that replays a run."""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import typing as t
from pathlib import Path

import pytest
from qgis.core import (
    QgsApplication,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsProcessingModelAlgorithm,
    QgsProcessingModelChildAlgorithm,
    QgsProcessingModelChildParameterSource,
    QgsProcessingModelParameter,
    QgsProcessingParameterFile,
    QgsProcessingParameterVectorLayer,
    QgsProject,
    QgsVectorLayer,
)

import topology_rules
from topology_rules.algorithms.rule import COUNT
from topology_rules.enums import PROVIDER_ID

from .helpers import (
    Ran,
    run_algorithm,
    saved_layer,
    start_algorithm,
    wait_for,
    write_toml,
)

CHAIN = ('LINESTRING(0 0, 10 0)', 'LINESTRING(10 0, 20 0)')
RING = ('LINESTRING(0 0, 10 0, 10 10, 0 0)',)
FAR = 'LINESTRING(0 50, 10 50)'
SECOND_FAR = 'LINESTRING(0 80, 10 80)'
DANGLES = 'line/must-not-have-dangles'
RULE = f'{PROVIDER_ID}:line_must_not_have_dangles'


@pytest.fixture(autouse=True)
def ready(executable: str, provider: object) -> None:
    """The real executable and the provider, as in a QGIS session."""


def project_layer(
    directory: Path, wkts: tuple[str, ...], name: str = 'roads'
) -> QgsVectorLayer:
    directory.mkdir(parents=True, exist_ok=True)
    layer = saved_layer(directory, wkts, name)
    QgsProject.instance().addMapLayer(layer)
    return layer


# Models


def native_or_skip(algorithm_id: str) -> None:
    registry = QgsApplication.processingRegistry()
    if registry.algorithmById(algorithm_id) is None:
        pytest.skip(f'{algorithm_id} is not in this QGIS')


def build_model(first_id: str, key: str, definition: t.Any) -> t.Any:
    model = QgsProcessingModelAlgorithm('chain', 'tests')
    model.addModelParameter(definition, QgsProcessingModelParameter(key))

    first = QgsProcessingModelChildAlgorithm(first_id)
    first.generateChildId(model)
    first.setDescription('check')
    first.addParameterSources(
        key, [QgsProcessingModelChildParameterSource.fromModelParameter(key)]
    )
    model.addChildAlgorithm(first)

    save = QgsProcessingModelChildAlgorithm('native:savefeatures')
    save.generateChildId(model)
    save.setDescription('save')
    save.addParameterSources(
        'INPUT',
        [
            QgsProcessingModelChildParameterSource.fromChildOutput(
                first.childId(), 'ERRORS_POINT'
            )
        ],
    )
    # The model names its destination parameter after the description, in
    # lower case
    save.addParameterSources(
        'OUTPUT',
        [
            QgsProcessingModelChildParameterSource.fromStaticValue(
                'TEMPORARY_OUTPUT'
            )
        ],
    )
    model.addChildAlgorithm(save)
    return model


def run_model(model: t.Any, parameters: dict[str, t.Any]) -> Ran:
    context = QgsProcessingContext()
    context.setProject(QgsProject.instance())
    feedback = QgsProcessingFeedback()
    results, ok = model.run(parameters, context, feedback)
    return Ran(results, ok, context, feedback.textLog())


def saved_count(ran: Ran) -> int:
    """The features that the save step of the model wrote."""
    results = ran.results['CHILD_RESULTS']
    key = next(
        name for name in results if name.startswith('native:savefeatures')
    )
    value = str(results[key]['OUTPUT'])
    layer = ran.context.getMapLayer(value) or QgsVectorLayer(
        value, 'saved', 'ogr'
    )
    assert layer.isValid(), results
    return int(layer.featureCount())


def rule_model() -> t.Any:
    native_or_skip('native:savefeatures')
    definition = QgsProcessingParameterVectorLayer('INPUT', 'Input layer')
    return build_model(RULE, 'INPUT', definition)


def test_a_model_passes_the_errors_to_the_next_step(tmp_path: Path):
    layer = project_layer(tmp_path, CHAIN)
    ran = run_model(rule_model(), {'INPUT': layer.id()})
    assert ran.ok, ran.log
    assert saved_count(ran) == 2


def test_a_model_with_a_clean_input_finishes(tmp_path: Path):
    layer = project_layer(tmp_path, RING)
    ran = run_model(rule_model(), {'INPUT': layer.id()})
    assert ran.ok, ran.log
    assert saved_count(ran) == 0


def definition_model() -> t.Any:
    native_or_skip('native:savefeatures')
    definition = QgsProcessingParameterFile('DEFINITION', 'Definition')
    return build_model(
        f'{PROVIDER_ID}:check_definition', 'DEFINITION', definition
    )


def definition_file(directory: Path, wkts: tuple[str, ...]) -> Path:
    data = directory / 'data'
    data.mkdir()
    saved_layer(data, wkts, 'roads')
    return write_toml(
        directory / 'topology.toml',
        {'roads': ('data/roads.gpkg', 'roads')},
        [(DANGLES, 'roads', None)],
    )


def test_a_model_can_start_with_check_definition(tmp_path: Path):
    definition = definition_file(tmp_path, CHAIN)
    ran = run_model(definition_model(), {'DEFINITION': str(definition)})
    assert ran.ok, ran.log
    assert saved_count(ran) == 2


def test_a_model_that_starts_with_check_definition_finishes_when_clean(
    tmp_path: Path,
):
    definition = definition_file(tmp_path, RING)
    ran = run_model(definition_model(), {'DEFINITION': str(definition)})
    assert ran.ok, ran.log
    assert saved_count(ran) == 0


# Batch


def three_layers(tmp_path: Path) -> list[tuple[QgsVectorLayer, int]]:
    return [
        (project_layer(tmp_path / 'a', CHAIN, 'first'), 2),
        (project_layer(tmp_path / 'b', (*CHAIN, FAR), 'second'), 4),
        (project_layer(tmp_path / 'c', (*CHAIN, FAR, SECOND_FAR), 'third'), 6),
    ]


def test_a_loop_of_processing_run_gives_the_counts(tmp_path: Path):
    import processing

    for index, (layer, count) in enumerate(three_layers(tmp_path)):
        results = processing.run(
            RULE,
            {
                'INPUT': layer.id(),
                'ERRORS_POINT': str(tmp_path / f'points_{index}.gpkg'),
            },
        )
        assert results[COUNT] == count


def test_a_loop_writes_one_file_for_each_run(tmp_path: Path):
    import processing

    for index, (layer, count) in enumerate(three_layers(tmp_path)):
        path = tmp_path / f'points_{index}.gpkg'
        processing.run(RULE, {'INPUT': layer.id(), 'ERRORS_POINT': str(path)})
        assert QgsVectorLayer(str(path), 'e', 'ogr').featureCount() == count


def test_three_tasks_at_once_give_the_counts_without_cross_talk(
    tmp_path: Path,
):
    layers = three_layers(tmp_path)
    started = [
        start_algorithm(
            'line_must_not_have_dangles',
            {
                'INPUT': layer.id(),
                'ERRORS_POINT': str(tmp_path / f'points_{index}.gpkg'),
            },
        )
        for index, (layer, _) in enumerate(layers)
    ]
    wait_for(lambda: all(run.done for run in started))
    for index, ((_, count), run) in enumerate(
        zip(layers, started, strict=True)
    ):
        ran = run.ran()
        assert ran.ok, ran.log
        assert ran.results[COUNT] == count
        path = tmp_path / f'points_{index}.gpkg'
        assert QgsVectorLayer(str(path), 'e', 'ogr').featureCount() == count


def test_three_tasks_at_once_on_one_layer_do_not_share_their_output(
    tmp_path: Path,
):
    layer = project_layer(tmp_path, CHAIN)
    started = [
        start_algorithm('line_must_not_have_dangles', {'INPUT': layer.id()})
        for _ in range(3)
    ]
    wait_for(lambda: all(run.done for run in started))
    counts = [run.ran().results[COUNT] for run in started]
    assert counts == [2, 2, 2]


# qgis_process


# How the program is named: as it is, with the extension of Windows, and as
# the batch files of conda-forge on Windows, which set up QGIS and start it
QGIS_PROCESS_NAMES = (
    'qgis_process',
    'qgis_process.exe',
    'qgis_process.bat',
    'qgis_process-qgis.bat',
    'qgis_process-qgis-ltr.bat',
)


def qgis_process() -> str | None:
    """The qgis_process of the QGIS that runs the tests, else one on the
    PATH."""
    prefix = Path(QgsApplication.prefixPath())
    folders = [
        prefix,
        prefix / 'bin',
        prefix / 'Contents' / 'MacOS',
        # Where conda puts the programs of QGIS on Windows
        prefix / 'Library' / 'bin',
    ]
    candidates = [os.environ.get('QGIS_PROCESS', '')]
    for name in QGIS_PROCESS_NAMES:
        candidates += [str(folder / name) for folder in folders]
        candidates.append(shutil.which(name) or '')
    return next(
        (path for path in candidates if path and Path(path).is_file()), None
    )


def link_program(folder: Path, executable: str) -> None:
    """Makes the tool available by the name that the plugin looks for. A copy
    where there are no links to files to be had, as on Windows."""
    if sys.platform == 'win32':
        shutil.copy(executable, folder / 'topology-checker.exe')
    else:
        (folder / 'topology-checker').symlink_to(executable)


@pytest.mark.slow
def test_qgis_process_runs_a_rule_of_the_plugin(
    tmp_path: Path, executable: str
):
    program = qgis_process()
    if program is None:
        pytest.skip('qgis_process is not installed')
    plugins = tmp_path / 'plugins'
    shutil.copytree(
        Path(topology_rules.__file__).parent,
        plugins / 'topology_rules',
        ignore=shutil.ignore_patterns('__pycache__'),
    )
    tools = tmp_path / 'bin'
    tools.mkdir()
    link_program(tools, executable)
    data = project_layer(tmp_path / 'data', CHAIN)
    source = data.source()
    output = tmp_path / 'errors.gpkg'
    env = {
        **os.environ,
        'QGIS_PLUGINPATH': str(plugins),
        'QGIS_CUSTOM_CONFIG_PATH': str(tmp_path / 'config'),
        'QT_QPA_PLATFORM': 'offscreen',
        'PATH': f'{tools}{os.pathsep}{os.environ["PATH"]}',
    }
    enable = subprocess.run(
        [program, 'plugins', 'enable', 'topology_rules'],
        env=env,
        capture_output=True,
        text=True,
        timeout=180,
        check=False,
    )
    assert enable.returncode == 0, enable.stdout + enable.stderr
    run = subprocess.run(
        [
            program,
            'run',
            RULE,
            f'--INPUT={source}',
            f'--ERRORS_POINT={output}',
        ],
        env=env,
        capture_output=True,
        text=True,
        timeout=180,
        check=False,
    )
    assert run.returncode == 0, run.stdout + run.stderr
    assert QgsVectorLayer(str(output), 'e', 'ogr').featureCount() == 2


# History


def test_the_python_command_of_a_run_replays_it(tmp_path: Path):
    import processing

    layer = project_layer(tmp_path, CHAIN)
    algorithm = QgsApplication.processingRegistry().createAlgorithmById(RULE)
    assert algorithm is not None
    parameters = {
        'INPUT': layer.id(),
        'TOLERANCE': 0.5,
        'ERRORS_POINT': str(tmp_path / 'points.gpkg'),
    }
    context = QgsProcessingContext()
    context.setProject(QgsProject.instance())
    command = algorithm.asPythonCommand(parameters, context)
    assert command.startswith(f'processing.run("{RULE}"')
    replayed = eval(command, {'processing': processing})
    assert replayed[COUNT] == 2


def test_the_command_has_the_parameters_that_were_given(tmp_path: Path):
    layer = project_layer(tmp_path, CHAIN)
    algorithm = QgsApplication.processingRegistry().createAlgorithmById(RULE)
    assert algorithm is not None
    context = QgsProcessingContext()
    context.setProject(QgsProject.instance())
    command = algorithm.asPythonCommand(
        {'INPUT': layer.id(), 'TOLERANCE': 0.5}, context
    )
    assert re.search(r"""['"]TOLERANCE['"]\s*:\s*0\.5""", command)
    assert 'INPUT' in command


def test_a_replayed_run_gives_the_same_counts_as_the_first(tmp_path: Path):
    import processing

    layer = project_layer(tmp_path, (*CHAIN, FAR))
    first = run_algorithm(
        'line_must_not_have_dangles',
        {'INPUT': layer.id(), 'ERRORS_POINT': str(tmp_path / 'one.gpkg')},
    )
    algorithm = QgsApplication.processingRegistry().createAlgorithmById(RULE)
    assert algorithm is not None
    context = QgsProcessingContext()
    context.setProject(QgsProject.instance())
    command = algorithm.asPythonCommand(
        {'INPUT': layer.id(), 'ERRORS_POINT': str(tmp_path / 'two.gpkg')},
        context,
    )
    second = eval(command, {'processing': processing})
    assert second[COUNT] == first.results[COUNT] == 4
