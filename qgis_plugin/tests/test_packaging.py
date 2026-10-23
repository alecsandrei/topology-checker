"""What the plugin folder ships, and what its metadata says to the plugin
repository."""

from __future__ import annotations

import configparser
import re
import subprocess
import sys
from pathlib import Path

import pytest

import topology_rules

PLUGIN = Path(topology_rules.__file__).parent
BINARY_SUFFIXES = {'.so', '.dylib', '.dll', '.exe', '.pyd', '.a', '.o', '.bin'}
# The first bytes of programs and libraries: ELF, Mach-O and Windows
BINARY_MAGIC = (
    b'\x7fELF',
    b'\xfe\xed\xfa\xce',
    b'\xfe\xed\xfa\xcf',
    b'\xce\xfa\xed\xfe',
    b'\xcf\xfa\xed\xfe',
    b'\xca\xfe\xba\xbe',
    b'MZ',
)
MAX_BYTES = 500_000
REQUIRED = (
    'name',
    'qgisMinimumVersion',
    'description',
    'about',
    'version',
    'author',
    'email',
    'repository',
    'tracker',
    'homepage',
    'icon',
)


def git(*arguments: str) -> subprocess.CompletedProcess[str] | None:
    """A git command in the plugin folder, or None where there is no git, as
    in the QGIS images."""
    try:
        return subprocess.run(
            ['git', *arguments],
            cwd=PLUGIN,
            capture_output=True,
            text=True,
            check=False,
        )
    except FileNotFoundError:
        return None


def shipped_files() -> list[Path]:
    """The files of the plugin folder that a commit would hold: what git
    tracks or would add, or else every file but the caches."""
    done = git('ls-files', '--cached', '--others', '--exclude-standard', '.')
    if done is not None and done.returncode == 0 and done.stdout.strip():
        return [PLUGIN / line for line in done.stdout.splitlines()]
    return [
        path
        for path in PLUGIN.rglob('*')
        if path.is_file() and '__pycache__' not in path.parts
    ]


@pytest.fixture(scope='module')
def files() -> list[Path]:
    found = shipped_files()
    assert found, 'no files in the plugin folder'
    return found


def recorded_modes() -> dict[Path, int]:
    """The mode that git records for each tracked file. A bind mount can show
    every file as executable, which the repository does not hold."""
    done = git('ls-files', '--stage', '.')
    if done is None or done.returncode != 0:
        return {}
    modes: dict[Path, int] = {}
    for line in done.stdout.splitlines():
        mode, _, rest = line.partition(' ')
        name = rest.split('\t', 1)[1]
        modes[PLUGIN / name] = int(mode, 8)
    return modes


def relative(path: Path) -> Path:
    return path.relative_to(PLUGIN)


def test_no_cache_is_shipped(files: list[Path]):
    bad = [
        str(relative(path))
        for path in files
        if '__pycache__' in path.parts or path.suffix in {'.pyc', '.pyo'}
    ]
    assert bad == []


def test_no_tests_or_settings_are_shipped(files: list[Path]):
    banned = {'tests', 'test', '.qgis-settings', '.venv', '.pytest_cache'}
    bad = [
        str(relative(path))
        for path in files
        if banned & set(relative(path).parts)
    ]
    assert bad == []


def test_no_binary_is_shipped_by_name(files: list[Path]):
    bad = [
        str(relative(p)) for p in files if p.suffix.lower() in BINARY_SUFFIXES
    ]
    assert bad == []


def test_no_binary_is_shipped_by_content(files: list[Path]):
    bad = []
    for path in files:
        with path.open('rb') as handle:
            head = handle.read(4)
        if any(head.startswith(magic) for magic in BINARY_MAGIC):
            bad.append(str(relative(path)))
    assert bad == []


@pytest.mark.skipif(sys.platform == 'win32', reason='no executable bit')
def test_no_file_is_executable(files: list[Path]):
    recorded = recorded_modes()
    bad = [
        str(relative(path))
        for path in files
        if (recorded[path] if path in recorded else path.stat().st_mode) & 0o111
    ]
    assert bad == []


def test_no_file_is_big(files: list[Path]):
    bad = [
        f'{relative(path)}: {path.stat().st_size} bytes'
        for path in files
        if path.stat().st_size > MAX_BYTES
    ]
    assert bad == []


def test_the_files_the_repository_needs_are_there(files: list[Path]):
    names = {relative(path).as_posix() for path in files}
    assert {
        '__init__.py',
        'metadata.txt',
        'LICENSE',
        'assets/icon.svg',
    } <= names


def test_the_plugin_has_a_class_factory():
    assert callable(topology_rules.classFactory)


# metadata.txt


@pytest.fixture(scope='module')
def metadata() -> configparser.ConfigParser:
    parser = configparser.ConfigParser(interpolation=None)
    parser.read(PLUGIN / 'metadata.txt', encoding='utf-8')
    return parser


def general(metadata: configparser.ConfigParser, key: str) -> str:
    return metadata['general'][key].strip()


def test_the_metadata_has_a_general_section(
    metadata: configparser.ConfigParser,
):
    assert metadata.has_section('general')


@pytest.mark.parametrize('key', REQUIRED)
def test_the_metadata_has_the_key_that_the_repository_needs(
    metadata: configparser.ConfigParser, key: str
):
    assert general(metadata, key)


def test_the_icon_of_the_metadata_exists(metadata: configparser.ConfigParser):
    assert (PLUGIN / general(metadata, 'icon')).is_file()


def test_the_oldest_qgis_is_3_34(metadata: configparser.ConfigParser):
    assert general(metadata, 'qgisMinimumVersion') == '3.34'


def test_qgis_4_is_supported(metadata: configparser.ConfigParser):
    assert general(metadata, 'supportsQt6').lower() == 'true'
    assert general(metadata, 'qgisMaximumVersion').startswith('4.')


def test_the_plugin_says_that_it_has_a_processing_provider(
    metadata: configparser.ConfigParser,
):
    assert general(metadata, 'hasProcessingProvider') == 'yes'


def test_the_about_text_says_how_to_get_the_tool(
    metadata: configparser.ConfigParser,
):
    about = general(metadata, 'about')
    assert 'topology-checker executable' in about
    assert '/qgis-plugin/#install' in about
    assert 'Processing' in about


def test_the_urls_of_the_metadata_are_web_addresses(
    metadata: configparser.ConfigParser,
):
    for key in ('repository', 'tracker', 'homepage'):
        assert re.match(r'https://\S+$', general(metadata, key)), key


def test_the_email_is_an_address(metadata: configparser.ConfigParser):
    assert re.fullmatch(r'\S+@\S+\.\S+', general(metadata, 'email'))


def test_the_description_is_one_line(metadata: configparser.ConfigParser):
    assert '\n' not in general(metadata, 'description')


def test_the_name_does_not_clash_with_the_core_plugin(
    metadata: configparser.ConfigParser,
):
    assert general(metadata, 'name') not in {
        'Topology Checker',
        'Topology Checker Pro',
    }
    assert PLUGIN.name != 'topology_checker'


def test_the_tags_include_topology(metadata: configparser.ConfigParser):
    tags = [tag.strip() for tag in general(metadata, 'tags').split(',')]
    assert 'topology' in tags


# The version of the plugin

WORKFLOW = PLUGIN.parent.parent / '.github' / 'workflows' / 'qgis-plugin.yml'


def test_the_version_is_a_release_version(metadata: configparser.ConfigParser):
    assert re.fullmatch(r'\d+\.\d+\.\d+', general(metadata, 'version'))


def test_the_version_is_the_one_of_the_project(
    metadata: configparser.ConfigParser,
):
    project = (PLUGIN.parent / 'pyproject.toml').read_text(encoding='utf-8')
    found = re.search(r'^version = "([^"]+)"', project, re.MULTILINE)
    assert found is not None
    assert general(metadata, 'version') == found.group(1)


def test_the_workflow_packages_on_version_tags():
    if not WORKFLOW.is_file():
        pytest.skip('the workflow is not in this folder')
    yaml = pytest.importorskip('yaml')
    config = yaml.safe_load(WORKFLOW.read_text(encoding='utf-8'))
    # YAML 1.1 reads the key `on` as true
    triggers = config.get('on', config.get(True))
    assert triggers['push']['tags'] == ['v*']


def test_the_package_job_takes_the_version_from_the_tag_without_its_v():
    if not WORKFLOW.is_file():
        pytest.skip('the workflow is not in this folder')
    text = WORKFLOW.read_text(encoding='utf-8')
    assert 'refs/tags/v*' in text
    assert '${GITHUB_REF_NAME#v}' in text
    assert 'qgis-plugin-v' not in text


# Files that QGIS leaves in the folder it is started from

DATABASES = {'.db', '.sqlite', '.sqlite3'}


def repository_files() -> list[Path]:
    """The files under qgis_plugin that git tracks or would add."""
    root = PLUGIN.parent
    try:
        done = subprocess.run(
            ['git', 'ls-files', '--cached', '--others', '--exclude-standard'],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
    except FileNotFoundError:
        pytest.skip('git is not installed')
    if done.returncode != 0:
        pytest.skip('the plugin is not in a repository')
    return [root / line for line in done.stdout.splitlines()]


def test_no_database_file_is_tracked_or_left_to_be_added():
    stray = [
        str(path.relative_to(PLUGIN.parent))
        for path in repository_files()
        if path.suffix.lower() in DATABASES
        and 'tests' not in path.relative_to(PLUGIN.parent).parts
    ]
    assert stray == []
