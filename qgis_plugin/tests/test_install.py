"""The install of the Python package from PyPI into the profile: its checks,
its folders and its logs, against a PyPI of the tests on 127.0.0.1."""

from __future__ import annotations

import ast
import collections.abc as c
import configparser
import dataclasses
import http.server
import importlib
import inspect
import os
import platform
import shlex
import subprocess
import sys
import threading
import time
import types
import typing as t
from pathlib import Path

import pytest
from qgis.core import QgsApplication, QgsNetworkAccessManager
from qgis.gui import QgisInterface
from qgis.PyQt.QtCore import QCoreApplication, QUrl

import topology_rules
from topology_rules.core import engine, folders, install, wheels
from topology_rules.exceptions import PackageError
from topology_rules.utils import not_none

from .helpers import wait_for
from .wheel_helpers import (
    LINUX,
    MARK,
    VERSION,
    WHEELS,
    answer,
    fake_module,
    mac,
    make_wheel,
    member_names,
    pretend_linux,
    release_file,
    sha256,
    wheel_name,
    windows,
)

# The index of the module, before a test points it to its own server
PRODUCTION = install.PYPI
FOLDER = folders.folder
# The modules of the install
MODULES = (install, wheels, folders)

# The PyPI of the tests

Route = c.Callable[[http.server.BaseHTTPRequestHandler], None]


class Index(http.server.ThreadingHTTPServer):
    daemon_threads = True
    # A slow answer is not waited for when the server closes
    block_on_close = False

    def __init__(self) -> None:
        super().__init__(('127.0.0.1', 0), Handler)
        self.routes: dict[str, Route] = {}
        self.asked: list[str] = []

    @property
    def url(self) -> str:
        return f'http://127.0.0.1:{self.server_address[1]}'

    def serve(
        self, path: str, content: bytes, status: int = 200, kind: str = ''
    ) -> str:
        """Answers the path with the content, and returns its URL."""

        def route(handler: http.server.BaseHTTPRequestHandler) -> None:
            handler.send_response(status)
            handler.send_header('Content-Type', kind or 'application/json')
            handler.send_header('Content-Length', str(len(content)))
            handler.end_headers()
            handler.wfile.write(content)

        self.routes[path] = route
        return self.url + path

    def release(self, data: object, version: str = VERSION) -> None:
        import json

        self.serve(
            f'/pypi/topology-checker/{version}/json', json.dumps(data).encode()
        )


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        index = t.cast(Index, self.server)
        index.asked.append(self.path)
        route = index.routes.get(self.path)
        if route is None:
            self.send_error(404, 'Not Found')
            return
        route(self)

    def log_message(self, format: str, *args: t.Any) -> None:
        pass


def trickle(handler: http.server.BaseHTTPRequestHandler) -> None:
    """A download that would take minutes."""
    handler.send_response(200)
    handler.send_header('Content-Type', 'application/octet-stream')
    handler.send_header('Content-Length', str(10_000_000))
    handler.end_headers()
    try:
        for _ in range(3000):
            handler.wfile.write(b'\0' * 1000)
            handler.wfile.flush()
            time.sleep(0.01)
    except OSError:
        pass


@pytest.fixture
def pypi(monkeypatch: pytest.MonkeyPatch) -> c.Iterator[Index]:
    index = Index()
    thread = threading.Thread(target=index.serve_forever, daemon=True)
    thread.start()
    monkeypatch.setattr(install, 'PYPI', f'{index.url}/pypi')
    yield index
    index.shutdown()
    index.server_close()


@pytest.fixture
def published(pypi: Index) -> tuple[Index, bytes]:
    """The release of the plugin's version, with a wheel for each platform
    of which only the Linux one is served."""
    content = make_wheel()
    files = []
    for kind, tag in WHEELS.items():
        body = content if kind == 'linux' else make_wheel(tag=tag)
        url = pypi.serve(f'/files/{wheel_name(tag)}', body, kind='x')
        files.append(release_file(tag, body, url))
    pypi.release(answer(files))
    return pypi, content


@pytest.fixture(autouse=True)
def linux(monkeypatch: pytest.MonkeyPatch) -> None:
    pretend_linux(monkeypatch)


@pytest.fixture(autouse=True)
def home(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> Path:
    """The folder of the package, in a profile of the test."""
    folder = tmp_path / 'profile' / 'python' / 'dependencies' / 'topology_rules'
    monkeypatch.setattr(folders, 'folder', lambda: folder)
    return folder


def package_modules() -> list[str]:
    return [
        name
        for name in sys.modules
        if name == wheels.MODULE or name.startswith(f'{wheels.MODULE}.')
    ]


@pytest.fixture(autouse=True)
def imports(monkeypatch: pytest.MonkeyPatch) -> c.Iterator[None]:
    """A sys.path of the test, and no topology_checker imported, as in a
    QGIS that never had it; what was imported before comes back after."""
    monkeypatch.setattr(sys, 'path', list(sys.path))
    for name in package_modules():
        monkeypatch.delitem(sys.modules, name)
    yield
    for name in package_modules():
        del sys.modules[name]
    importlib.invalidate_caches()


@pytest.fixture
def messages() -> c.Iterator[list[str]]:
    """What reaches the Topology Rules tab of the Log Messages panel."""
    found: list[str] = []

    def received(message: str, tag: str, *rest: object) -> None:
        if tag == folders.TAG:
            found.append(message)

    log = not_none(QgsApplication.messageLog())
    # The signal that the panel listens to, which QGIS 4 renamed
    signal = getattr(log, 'messageReceivedWithFormat', log.messageReceived)
    signal.connect(received)
    yield found
    signal.disconnect(received)


def version_folder(
    home: Path,
    version: str,
    tag: str = LINUX,
    holds: str | None = None,
) -> Path:
    """A version folder as an install or pip leaves it."""
    folder = home / version
    package = folder / 'topology_checker'
    package.mkdir(parents=True)
    (package / '__init__.py').write_text(f'MARK = {version!r}\n')
    info = folder / f'topology_checker-{holds or version}.dist-info'
    info.mkdir()
    (info / 'METADATA').write_text(
        f'Metadata-Version: 2.4\nName: topology-checker\n'
        f'Version: {holds or version}\n'
    )
    (info / 'WHEEL').write_text(f'Wheel-Version: 1.0\nTag: {tag}\n')
    return folder


def load_from(
    folder: Path, monkeypatch: pytest.MonkeyPatch
) -> types.ModuleType:
    """A topology_checker imported from a version folder."""
    loaded = fake_module('topology_checker', folder.name)
    loaded.__file__ = str(folder / 'topology_checker' / '__init__.py')
    monkeypatch.setitem(sys.modules, 'topology_checker', loaded)
    return loaded


def lock_libraries(monkeypatch: pytest.MonkeyPatch) -> None:
    """Deleting a library fails, as on Windows while it is loaded."""
    unlink = os.unlink

    def locked(path: t.Any, *args: t.Any, **kwargs: t.Any) -> None:
        if str(path).endswith('.pyd'):
            raise PermissionError(13, 'Access is denied', str(path))
        unlink(path, *args, **kwargs)

    monkeypatch.setattr(os, 'unlink', locked)


@dataclasses.dataclass
class Outcome:
    lines: list[str] = dataclasses.field(default_factory=list)
    found: list[install.Release] = dataclasses.field(default_factory=list)
    done: list[str] = dataclasses.field(default_factory=list)
    failed: list[str] = dataclasses.field(default_factory=list)

    @property
    def over(self) -> bool:
        return bool(self.found or self.done or self.failed)


def start(task: install.Task) -> Outcome:
    outcome = Outcome()
    task.logged.connect(outcome.lines.append)
    task.failed.connect(outcome.failed.append)
    if isinstance(task, install.ReleaseTask):
        task.found.connect(outcome.found.append)
    if isinstance(task, install.InstallTask):
        task.done.connect(outcome.done.append)
    not_none(QgsApplication.taskManager()).addTask(task)
    return outcome


def run(task: install.Task) -> Outcome:
    """Runs a task to its end, as the panel does."""
    outcome = start(task)
    wait_for(lambda: outcome.over, 30)
    for _ in range(5):
        QgsApplication.processEvents()
    return outcome


def find(version: str | None = None) -> install.Release:
    outcome = run(install.ReleaseTask(version))
    assert outcome.failed == []
    return outcome.found[0]


def leftovers(home: Path) -> list[str]:
    return sorted(path.name for path in home.glob('.*'))


# The module


def test_pypi_is_the_index_by_default():
    assert PRODUCTION == 'https://pypi.org/pypi'


def test_the_folder_is_in_the_profile():
    profile = Path(QgsApplication.qgisSettingsDirPath())
    assert FOLDER() == profile / 'python' / 'dependencies' / 'topology_rules'


def test_the_wanted_version_is_that_of_the_plugin():
    parser = configparser.ConfigParser(interpolation=None)
    parser.read(folders.METADATA, encoding='utf-8')
    assert install.wanted() == parser['general']['version']


@pytest.mark.parametrize('module', MODULES)
def test_the_install_fetches_without_pip_or_other_programs(
    module: types.ModuleType,
):
    tree = ast.parse(Path(str(module.__file__)).read_text(encoding='utf-8'))
    imported = {
        alias.name.split('.')[0]
        for node in ast.walk(tree)
        if isinstance(node, ast.Import)
        for alias in node.names
    } | {
        (node.module or '').split('.')[0]
        for node in ast.walk(tree)
        if isinstance(node, ast.ImportFrom)
    }
    assert not imported & {'subprocess', 'urllib', 'requests', 'http', 'pip'}


def test_the_panel_finds_what_it_uses_in_install():
    for name in install.__all__:
        assert hasattr(install, name), name
    assert {
        'folder',
        'activate',
        'unsupported',
        'wanted',
        'installed',
        'outdated',
        'restart_needed',
        'command',
        'terminal',
        'remove',
        'size_text',
        'log',
        'Release',
        'Task',
        'ReleaseTask',
        'InstallTask',
    } <= set(install.__all__)


@pytest.mark.parametrize('module', MODULES)
def test_each_function_of_the_install_says_what_it_does(
    module: types.ModuleType,
):
    unsaid = [
        name
        for name, function in inspect.getmembers(module, inspect.isfunction)
        if function.__module__ == module.__name__ and not function.__doc__
    ]
    assert unsaid == []


def test_a_release_goes_into_the_folder_of_its_version(home: Path):
    release = install.Release(
        VERSION, wheel_name(LINUX), 'https://files.invalid/x', 1, sha256(b'')
    )
    assert install.folder_of(release.version) == home / VERSION
    assert install.InstallTask(release).target == home / VERSION


# Why this QGIS cannot use the package


def test_a_qgis_that_cannot_use_it_asks_pypi_nothing(
    pypi: Index, monkeypatch: pytest.MonkeyPatch, home: Path
):
    monkeypatch.setattr(platform, 'machine', lambda: 'aarch64')
    outcome = run(install.ReleaseTask())
    assert outcome.failed == ['No wheel for Linux aarch64']
    assert outcome.found == []
    assert not any(line.startswith('No other') for line in outcome.lines)
    assert pypi.asked == []
    assert not (home / VERSION).exists()


# PyPI


def test_the_release_is_found_before_anything_is_downloaded(
    published: tuple[Index, bytes],
):
    pypi, content = published
    release = find()
    assert release.filename == wheel_name(LINUX)
    assert release.sha256 == sha256(content)
    assert release.size == len(content)
    assert pypi.asked == [f'/pypi/topology-checker/{VERSION}/json']


def test_a_version_that_pypi_does_not_have_fails(pypi: Index):
    outcome = run(install.ReleaseTask())
    assert outcome.found == []
    assert outcome.failed == [f'PyPI has no topology-checker {VERSION}']


def test_a_server_error_fails_with_its_status(pypi: Index):
    pypi.serve(f'/pypi/topology-checker/{VERSION}/json', b'', status=503)
    outcome = run(install.ReleaseTask())
    assert outcome.failed == [
        f'Cannot fetch {pypi.url}/pypi/topology-checker/{VERSION}/json: '
        'HTTP 503 Service Unavailable'
    ]


def test_a_server_error_after_an_answer_fails_with_its_status(pypi: Index):
    # QGIS keeps answers, and gives the one of before to a request that fails
    pypi.release(answer([]))
    run(install.ReleaseTask())
    pypi.serve(f'/pypi/topology-checker/{VERSION}/json', b'', status=503)
    outcome = run(install.ReleaseTask())
    assert outcome.failed == [
        f'Cannot fetch {pypi.url}/pypi/topology-checker/{VERSION}/json: '
        'HTTP 503 Service Unavailable'
    ]


def test_downloads_are_kept_out_of_the_network_cache(
    published: tuple[Index, bytes],
):
    pypi, _ = published
    release = find()
    outcome = run(install.InstallTask(release))
    assert outcome.done == [VERSION]
    cache = not_none(QgsNetworkAccessManager.instance().cache())
    for url in (
        f'{pypi.url}/pypi/topology-checker/{VERSION}/json',
        release.url,
    ):
        assert not cache.metaData(QUrl(url)).isValid(), url


def test_no_network_fails_with_the_error(
    monkeypatch: pytest.MonkeyPatch,
):
    # A port that nothing listens on
    closed = Index()
    port = closed.server_address[1]
    closed.server_close()
    monkeypatch.setattr(install, 'PYPI', f'http://127.0.0.1:{port}/pypi')
    outcome = run(install.ReleaseTask())
    [error] = outcome.failed
    assert error.startswith(f'Cannot fetch http://127.0.0.1:{port}/pypi')
    assert 'refused' in error.lower()


@pytest.mark.slow
def test_downloads_end_while_the_main_thread_holds_the_gil(tmp_path: Path):
    # A hang would stop the tests, so the fetches run in a QGIS of their own
    script = Path(__file__).with_name('fetch_under_load.py')
    plugin = Path(topology_rules.__file__).parents[1]
    env = {
        **os.environ,
        # The tests drop both for the fake executables
        'PYTHONHOME': sys.base_prefix,
        'PYTHONPATH': os.pathsep.join(sys.path),
        'QGIS_CUSTOM_CONFIG_PATH': str(tmp_path),
        'QT_QPA_PLATFORM': 'offscreen',
    }
    done = subprocess.run(
        [sys.executable, str(script), str(plugin), '30'],
        env=env,
        capture_output=True,
        text=True,
        timeout=300,
        check=False,
    )
    assert done.returncode == 0, done.stderr[-4000:]
    assert done.stdout.splitlines()[-1] == '30'


def test_an_answer_that_is_not_json_fails(pypi: Index):
    pypi.serve(
        f'/pypi/topology-checker/{VERSION}/json', b'<html>', kind='text/html'
    )
    outcome = run(install.ReleaseTask())
    [error] = outcome.failed
    assert error.startswith('PyPI sent no JSON: ')


def test_json_without_files_fails(pypi: Index):
    pypi.release({'info': {'version': VERSION}})
    outcome = run(install.ReleaseTask())
    assert outcome.failed == ['PyPI sent no files']


def test_a_wheel_that_is_gone_fails(pypi: Index, home: Path):
    content = make_wheel()
    pypi.release(answer([release_file(LINUX, content, f'{pypi.url}/gone')]))
    outcome = run(install.InstallTask(find()))
    assert outcome.failed == [
        f'Cannot fetch {pypi.url}/gone: HTTP 404 Not Found'
    ]
    assert not (home / VERSION).exists()


# The install


def test_a_first_install_loads_without_a_restart(
    published: tuple[Index, bytes], home: Path
):
    outcome = run(install.InstallTask(find()))
    assert outcome.failed == []
    assert outcome.done == [VERSION]
    target = home / VERSION
    assert sys.path[0] == str(target)
    module = importlib.import_module('topology_checker')
    assert module.MARK == MARK
    assert Path(str(module.__file__)).parent == target / 'topology_checker'
    assert install.installed() == VERSION
    assert not install.restart_needed()
    assert leftovers(home) == []


def test_only_the_package_is_installed(pypi: Index, home: Path):
    content = make_wheel(
        extra={
            'shapely/__init__.py': '',
            f'topology_checker-{VERSION}.data/scripts/tool': '',
        }
    )
    url = pypi.serve('/files/wheel.whl', content)
    pypi.release(answer([release_file(LINUX, content, url)]))
    outcome = run(install.InstallTask(find()))
    assert outcome.done == [VERSION]
    assert sorted(path.name for path in (home / VERSION).iterdir()) == [
        'topology_checker',
        f'topology_checker-{VERSION}.dist-info',
    ]
    assert 'Left out shapely' in outcome.lines


def test_a_wheel_with_another_sha256_is_not_unpacked(pypi: Index, home: Path):
    content = make_wheel()
    url = pypi.serve('/files/wheel.whl', content + b'tampered')
    pypi.release(answer([release_file(LINUX, content, url)]))
    outcome = run(install.InstallTask(find()))
    [error] = outcome.failed
    tampered = sha256(content + b'tampered')
    assert error == 'The download does not match its SHA-256 on PyPI'
    assert f'SHA-256 {tampered}, PyPI has {sha256(content)}' in outcome.lines
    assert outcome.done == []
    assert not (home / VERSION).exists()
    assert leftovers(home) == []


@pytest.mark.parametrize(
    'name',
    [
        '../evil.py',
        'topology_checker/../../evil.py',
        '/evil.py',
        '\\evil.py',
        'C:/evil.py',
        'topology_checker\\..\\..\\evil.py',
    ],
)
def test_a_wheel_with_a_file_outside_its_folder_is_refused(
    pypi: Index, home: Path, tmp_path: Path, name: str
):
    content = make_wheel(extra={name: 'print("escaped")'})
    # Stored as given on each system, and read with a slash for each
    # backslash on Windows only
    assert name.encode() in content
    [read] = [found for found in member_names(content) if 'evil' in found]
    assert read == (name.replace('\\', '/') if os.sep == '\\' else name)
    url = pypi.serve('/files/wheel.whl', content)
    pypi.release(answer([release_file(LINUX, content, url)]))
    outcome = run(install.InstallTask(find()))
    assert outcome.failed == [f'Refused {read}: outside of the folder']
    assert not (home / VERSION).exists()
    assert leftovers(home) == []
    assert list(tmp_path.rglob('evil.py')) == []


def test_a_wheel_that_is_no_zip_is_refused(pypi: Index, home: Path):
    content = b'not a zip'
    url = pypi.serve('/files/wheel.whl', content)
    pypi.release(answer([release_file(LINUX, content, url)]))
    outcome = run(install.InstallTask(find()))
    [error] = outcome.failed
    assert error.startswith('Not a wheel: ')
    assert not (home / VERSION).exists()


def test_a_package_that_does_not_import_is_taken_out(pypi: Index, home: Path):
    content = make_wheel(
        extra={'topology_checker/__init__.py': 'raise ImportError("broken")'}
    )
    url = pypi.serve('/files/wheel.whl', content)
    pypi.release(answer([release_file(LINUX, content, url)]))
    outcome = run(install.InstallTask(find()))
    assert outcome.failed == [
        f'topology-checker {VERSION} does not load: broken'
    ]
    assert outcome.done == []
    assert not (home / VERSION).exists()
    assert str(home / VERSION) not in sys.path
    assert 'topology_checker' not in sys.modules


def test_an_install_can_be_cancelled(pypi: Index, home: Path):
    url = pypi.url + '/files/slow.whl'
    pypi.routes['/files/slow.whl'] = trickle
    release = install.Release(
        VERSION, wheel_name(LINUX), url, 10_000_000, sha256(b'')
    )
    task = install.InstallTask(release)
    outcome = start(task)
    wait_for(lambda: task.progress() > 0, 30)
    started = time.monotonic()
    task.cancel()
    wait_for(lambda: outcome.over, 30)
    QgsApplication.processEvents()
    assert time.monotonic() - started < 10
    assert outcome.failed == ['Cancelled']
    assert outcome.lines[-1] == 'Cancelled'
    assert not (home / VERSION).exists()
    assert leftovers(home) == []


def test_finding_the_release_can_be_cancelled(pypi: Index):
    pypi.routes[f'/pypi/topology-checker/{VERSION}/json'] = trickle
    task = install.ReleaseTask()
    outcome = start(task)
    wait_for(lambda: bool(pypi.asked), 30)
    task.cancel()
    wait_for(lambda: outcome.over, 30)
    assert outcome.failed == ['Cancelled']
    assert outcome.found == []


def test_an_upgrade_loads_after_a_restart(
    published: tuple[Index, bytes], home: Path, monkeypatch: pytest.MonkeyPatch
):
    old = version_folder(home, '0.0.1')
    loaded = load_from(old, monkeypatch)
    outcome = run(install.InstallTask(find()))
    assert outcome.done == [VERSION]
    assert outcome.lines[-1] == (
        f'Restart QGIS to load topology-checker {VERSION}, as the one in '
        f'{old / "topology_checker"} is loaded'
    )
    assert sys.modules['topology_checker'] is loaded
    assert sys.path[0] == str(home / VERSION)
    assert install.installed() == VERSION
    assert install.restart_needed()


def test_the_same_version_is_installed_again_over_a_broken_one(
    published: tuple[Index, bytes], home: Path
):
    broken = home / VERSION / 'topology_checker'
    broken.mkdir(parents=True)
    (broken / 'stray.py').write_text('')
    outcome = run(install.InstallTask(find()))
    assert outcome.done == [VERSION]
    assert not (broken / 'stray.py').exists()
    assert install.installed() == VERSION


def test_a_loaded_package_is_left_as_it_is(
    published: tuple[Index, bytes],
    home: Path,
    monkeypatch: pytest.MonkeyPatch,
):
    pypi, _ = published
    folder = version_folder(home, VERSION)
    library = folder / 'topology_checker' / '_native.pyd'
    library.write_text('')
    before = sorted(path.relative_to(folder) for path in folder.rglob('*'))
    load_from(folder, monkeypatch)
    lock_libraries(monkeypatch)
    outcome = run(install.InstallTask(find()))
    assert outcome.failed == [
        f'Cannot replace {folder} while it is loaded. Restart QGIS and '
        'install again'
    ]
    assert outcome.done == []
    assert sorted(p.relative_to(folder) for p in folder.rglob('*')) == before
    assert install.installed() == VERSION
    # Refused before the download
    assert pypi.asked == [f'/pypi/topology-checker/{VERSION}/json']
    assert leftovers(home) == []


def test_a_package_loaded_during_the_download_is_left_as_it_is(
    home: Path, monkeypatch: pytest.MonkeyPatch
):
    folder = version_folder(home, VERSION)
    before = sorted(path.relative_to(folder) for path in folder.rglob('*'))
    unpacked = home / f'.{VERSION}-a1b2'
    (unpacked / 'topology_checker').mkdir(parents=True)
    load_from(folder, monkeypatch)
    with pytest.raises(wheels.Failed, match='while it is loaded'):
        folders.place(unpacked, folder)
    assert sorted(p.relative_to(folder) for p in folder.rglob('*')) == before
    assert not unpacked.exists()


# The log


def test_each_step_is_logged_live_in_qgis_and_in_the_folder(
    published: tuple[Index, bytes], home: Path, messages: list[str]
):
    pypi, content = published
    found = run(install.ReleaseTask())
    release = found.found[0]
    installed = run(install.InstallTask(release))
    name = wheel_name(LINUX)
    size = install.size_text(len(content))
    digest = sha256(content)
    expected = [
        'Python 3.12.8 on Linux x86_64 glibc 2.35',
        f'Fetching {pypi.url}/pypi/topology-checker/{VERSION}/json',
        f'Found {name} ({size}), SHA-256 {digest}',
        f'Downloading {name} ({size})',
        f'From {pypi.url}/files/{name}',
        f'SHA-256 {digest} matches PyPI',
        f'Unpacked 4 files into {home / VERSION}',
        'No other packages installed.',
        f'Loaded topology-checker {VERSION}',
    ]
    assert found.lines + installed.lines == expected
    wait_for(lambda: len(messages) >= len(expected), 10)
    assert messages == expected
    written = (home / 'install.log').read_text(encoding='utf-8').splitlines()
    # Each line of the file starts with the date and the time
    assert [line.split(' ', 2)[2] for line in written] == expected


def test_a_failure_is_logged_with_its_error(
    pypi: Index, home: Path, messages: list[str]
):
    outcome = run(install.ReleaseTask())
    [error] = outcome.failed
    assert outcome.lines[-1] == error
    wait_for(lambda: error in messages, 10)
    assert error in (home / 'install.log').read_text(encoding='utf-8')


def test_sizes_are_in_words():
    assert install.size_text(3_100_000) == '3.1 MB'
    assert install.size_text(2_000) == '2 KB'
    assert install.size_text(10) == '1 KB'


# The folders at plugin load


def test_activate_loads_the_installed_version(home: Path):
    target = version_folder(home, VERSION)
    install.activate()
    assert sys.path[0] == str(target)
    assert importlib.import_module('topology_checker').MARK == VERSION


def test_activate_without_an_install_changes_nothing(home: Path):
    before = list(sys.path)
    install.activate()
    assert sys.path == before
    assert not home.exists()


def test_activate_twice_lists_the_folder_once(home: Path):
    target = version_folder(home, VERSION)
    install.activate()
    install.activate()
    assert sys.path.count(str(target)) == 1


def test_activate_deletes_what_is_not_used(home: Path, messages: list[str]):
    version_folder(home, '0.0.1')
    kept = version_folder(home, VERSION)
    unfinished = home / f'.{VERSION}-a1b2'
    (unfinished / 'topology_checker').mkdir(parents=True)
    # What a removal leaves when Windows keeps the library
    removed = home / '0.0.2' / 'topology_checker'
    removed.mkdir(parents=True)
    (home / 'install.log').write_text('')
    install.activate()
    assert sorted(path.name for path in home.iterdir()) == [
        VERSION,
        'install.log',
    ]
    assert sys.path[0] == str(kept)
    wait_for(lambda: len(messages) == 3, 10)
    assert sorted(messages) == sorted(
        [
            f'Removed {unfinished}: left by an install that did not finish',
            f'Removed {home / "0.0.1"}: replaced by {VERSION}',
            f'Removed {home / "0.0.2"}: no dist-info',
        ]
    )


def test_activate_keeps_the_folder_of_a_loaded_package(
    home: Path, monkeypatch: pytest.MonkeyPatch
):
    old = version_folder(home, '0.0.1')
    version_folder(home, VERSION)
    load_from(old, monkeypatch)
    install.activate()
    assert old.exists()


@pytest.mark.parametrize(
    ('tag', 'holds'),
    [
        ('cp310-abi3-win_amd64', None),
        ('cp310-abi3-macosx_11_0_arm64', None),
        ('cp313-cp313-manylinux_2_17_x86_64', None),
        ('cp314-abi3-manylinux_2_17_x86_64', None),
        (LINUX, '0.0.9'),
    ],
)
def test_a_folder_for_another_python_or_platform_is_not_loaded(
    home: Path, tag: str, holds: str | None
):
    folder = version_folder(home, VERSION, tag, holds)
    install.activate()
    assert str(folder) not in sys.path
    assert install.installed() is None
    # It may be what another QGIS of the profile uses
    assert folder.exists()


def test_a_folder_newer_than_the_plugin_is_not_loaded(home: Path):
    folder = version_folder(home, '99.0.0')
    install.activate()
    assert str(folder) not in sys.path
    assert install.installed() is None


def test_loading_the_plugin_activates_the_package(
    home: Path, qgis_iface: QgisInterface
):
    target = version_folder(home, VERSION)
    topology_rules.classFactory(qgis_iface)
    assert sys.path[0] == str(target)


# Versions and updates


def test_nothing_installed(home: Path):
    assert install.installed() is None
    assert not install.outdated()
    assert not install.restart_needed()


def test_the_installed_version_is_that_of_the_folder(home: Path):
    version_folder(home, VERSION)
    assert install.installed() == VERSION
    assert not install.outdated()


def test_an_older_install_is_offered_an_update(
    home: Path, monkeypatch: pytest.MonkeyPatch
):
    version_folder(home, '0.2.1')
    monkeypatch.setattr(folders, 'wanted', lambda: '0.10.0')
    assert install.installed() == '0.2.1'
    assert install.outdated()


def test_the_newest_install_is_used(
    home: Path, monkeypatch: pytest.MonkeyPatch
):
    version_folder(home, '0.2.0')
    version_folder(home, '0.10.0')
    monkeypatch.setattr(folders, 'wanted', lambda: '0.10.0')
    assert install.installed() == '0.10.0'


def test_a_package_from_elsewhere_needs_no_restart(
    home: Path, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    loaded = fake_module('topology_checker', '0.2.1')
    loaded.__file__ = str(tmp_path / 'site-packages' / 'topology_checker.py')
    monkeypatch.setitem(sys.modules, 'topology_checker', loaded)
    assert not install.restart_needed()
    version_folder(home, VERSION)
    assert install.restart_needed()


def test_a_package_too_old_that_is_loaded_says_to_restart(
    home: Path, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    """The package that QGIS loaded stays loaded when another is installed,
    until QGIS restarts."""
    loaded = fake_module('topology_checker', '0.2.1')
    loaded.__file__ = str(tmp_path / 'site-packages' / 'topology_checker.py')
    monkeypatch.setitem(sys.modules, 'topology_checker', loaded)
    with pytest.raises(PackageError) as before:
        engine.find_package()
    assert str(before.value).endswith('it has no Topology.add_layer.')
    version_folder(home, VERSION)
    with pytest.raises(PackageError) as after:
        engine.find_package()
    assert str(after.value) == (
        f'{before.value} Restart QGIS to load {VERSION}.'
    )


# Remove


def test_remove_deletes_the_folder_and_takes_it_off_the_path(
    home: Path, messages: list[str]
):
    version_folder(home, VERSION)
    install.activate()
    (home / 'install.log').write_text('')
    install.remove()
    assert not home.exists()
    assert not any(str(home) in entry for entry in sys.path)
    assert install.installed() is None
    wait_for(lambda: bool(messages), 10)
    assert messages == [f'Removed {home}']


def test_remove_leaves_a_loaded_package_loaded(home: Path):
    version_folder(home, VERSION)
    install.activate()
    module = importlib.import_module('topology_checker')
    install.remove()
    assert sys.modules['topology_checker'] is module
    assert install.restart_needed()


def test_remove_without_an_install_does_nothing(home: Path):
    install.remove()
    assert not home.exists()


def test_what_a_locked_library_keeps_goes_at_the_next_start(
    home: Path, monkeypatch: pytest.MonkeyPatch, messages: list[str]
):
    folder = version_folder(home, VERSION)
    library = folder / 'topology_checker' / '_native.pyd'
    library.write_text('')
    unlink = os.unlink
    lock_libraries(monkeypatch)
    install.remove()
    assert library.exists()
    assert list(folder.glob('*.dist-info')) == []
    assert install.installed() is None
    monkeypatch.setattr(os, 'unlink', unlink)
    install.activate()
    assert not folder.exists()
    wait_for(lambda: len(messages) == 2, 10)
    assert messages[0].startswith(f'Could not remove all of {home}: ')
    assert messages[1] == f'Removed {folder}: no dist-info'


# The command that does the same


def test_the_command_on_linux(home: Path):
    # Quoted, as the folder of the tests may hold a backslash or a colon
    target = shlex.quote(str(home / VERSION))
    assert install.command() == (
        f'python3 -m pip install --no-deps --target {target} '
        f'topology-checker=={VERSION}'
    )
    assert install.terminal() == 'Terminal'


def test_the_command_on_windows(home: Path, monkeypatch: pytest.MonkeyPatch):
    windows(monkeypatch, 'win-amd64')
    assert install.command() == (
        f'python -m pip install --no-deps --target "{home / VERSION}" '
        f'topology-checker=={VERSION}'
    )
    assert install.terminal() == 'OSGeo4W Shell'


def test_the_command_on_macos_names_the_python_of_the_app(
    home: Path, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    mac(monkeypatch, 'arm64', '15.7')
    contents = tmp_path / 'QGIS 4.app' / 'Contents'
    python = contents / 'MacOS' / 'python3.12'
    python.parent.mkdir(parents=True)
    python.write_text('')
    monkeypatch.setattr(
        QCoreApplication, 'applicationDirPath', lambda: str(python.parent)
    )
    monkeypatch.setattr(sys, 'base_prefix', str(contents / 'Frameworks'))
    target = shlex.quote(str(home / VERSION))
    assert install.command() == (
        f"PYTHONHOME='{contents / 'Frameworks'}' '{python}' -m pip install "
        f'--no-deps --target {target} topology-checker=={VERSION}'
    )
    assert install.terminal() == 'Terminal'
    # The script of the app that sets the home, where there is one
    script = python.parent / 'python'
    script.write_text('')
    assert install.command() == (
        f"'{script}' -m pip install --no-deps --target {target} "
        f'topology-checker=={VERSION}'
    )


def test_the_command_quotes_a_folder_with_spaces(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    folder = tmp_path / 'Application Support' / 'topology_rules'
    monkeypatch.setattr(folders, 'folder', lambda: folder)
    assert f"--target '{folder / VERSION}' " in install.command()


# What the plugin repository shows


GUIDE = 'https://alecsandrei.github.io/topology-checker/stable/qgis-plugin/'


def test_the_about_text_names_the_package_and_links_the_guide():
    parser = configparser.ConfigParser(interpolation=None)
    parser.read(folders.METADATA, encoding='utf-8')
    about = parser['general']['about']
    assert 'topology-checker Python package' in about
    assert f'{GUIDE}#install' in about
    assert 'shapely' not in about
    assert 'pyproj' not in about


def test_the_guide_has_the_command_for_each_system():
    page = folders.METADATA.parents[2] / 'docs' / 'qgis-plugin.md'
    if not page.is_file():
        pytest.skip('The docs are not next to the plugin')
    text = page.read_text(encoding='utf-8')
    assert '\n## Install\n' in text
    assert 'Installs nothing else' in text
    assert 'shapely' not in text
    assert 'pyproj' not in text
    for system in ('Windows, in the OSGeo4W Shell', 'macOS', 'Linux'):
        assert f'\n{system}' in text
    assert text.count('-m pip install --no-deps --target') == 3
