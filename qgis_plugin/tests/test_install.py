"""The install of the Python package from PyPI into the profile: the wheel
that fits, its checks, its folders and its logs, against a PyPI of the tests
on 127.0.0.1."""

from __future__ import annotations

import ast
import collections.abc as c
import configparser
import dataclasses
import hashlib
import http.server
import importlib
import io
import os
import platform
import sys
import sysconfig
import threading
import time
import types
import typing as t
import zipfile
from pathlib import Path

import pytest
from qgis.core import QgsApplication
from qgis.gui import QgisInterface
from qgis.PyQt.QtCore import QCoreApplication

import topology_rules
from topology_rules.core import install

from .helpers import wait_for

VERSION = install.wanted()
# The index of the module, before a test points it to its own server
PRODUCTION = install.PYPI
FOLDER = install.folder
LINUX = 'cp310-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64'
WHEELS = {
    'windows': 'cp310-abi3-win_amd64',
    'mac_intel': 'cp310-abi3-macosx_10_12_x86_64',
    'mac_arm': 'cp310-abi3-macosx_11_0_arm64',
    'linux': LINUX,
}
MARK = 'from the wheel of the tests'


def wheel_name(tag: str, version: str = VERSION) -> str:
    return f'topology_checker-{version}-{tag}.whl'


def make_wheel(
    version: str = VERSION,
    tag: str = LINUX,
    extra: dict[str, str] | None = None,
) -> bytes:
    """A wheel of the package as maturin builds it, with a pure Python
    module in place of the library."""
    info = f'topology_checker-{version}.dist-info'
    tags = ''.join(
        f'Tag: {python}-{abi}-{plat}\n'
        for python, abi, plats in [tag.split('-')]
        for plat in plats.split('.')
    )
    members = {
        'topology_checker/__init__.py': f'MARK = {MARK!r}\n',
        f'{info}/METADATA': (
            f'Metadata-Version: 2.4\nName: topology-checker\n'
            f'Version: {version}\nRequires-Dist: shapely>=2,<3\n'
        ),
        f'{info}/WHEEL': (
            'Wheel-Version: 1.0\nGenerator: maturin (1.15.0)\n'
            f'Root-Is-Purelib: false\n{tags}'
        ),
        f'{info}/RECORD': '',
        **(extra or {}),
    }
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, 'w') as archive:
        for name, text in members.items():
            archive.writestr(name, text)
    return buffer.getvalue()


def sha256(content: bytes) -> str:
    return hashlib.sha256(content).hexdigest()


def release_file(
    tag: str, content: bytes, url: str, version: str = VERSION
) -> dict[str, t.Any]:
    """A file of a release, as the JSON of PyPI lists it."""
    return {
        'filename': wheel_name(tag, version),
        'url': url,
        'size': len(content),
        'digests': {'sha256': sha256(content)},
        'packagetype': 'bdist_wheel',
    }


def answer(files: list[dict[str, t.Any]], version: str = VERSION) -> dict:
    sdist = {
        'filename': f'topology_checker-{version}.tar.gz',
        'url': 'https://example.invalid/sdist',
        'size': 1,
        'digests': {'sha256': sha256(b'sdist')},
        'packagetype': 'sdist',
    }
    return {
        'info': {'version': version, 'requires_python': '>=3.10'},
        'urls': [*files, sdist],
    }


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


# This QGIS: Linux x86_64 with glibc 2.35, Python 3.12.8, shapely 2.0.6 and
# pyproj 3.7.0, whatever the machine of the tests


def fake_module(name: str, version: str) -> types.ModuleType:
    module = types.ModuleType(name)
    module.__version__ = version  # type: ignore[attr-defined]
    return module


@pytest.fixture(autouse=True)
def linux(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(platform, 'system', lambda: 'Linux')
    monkeypatch.setattr(platform, 'machine', lambda: 'x86_64')
    monkeypatch.setattr(platform, 'libc_ver', lambda: ('glibc', '2.35'))
    monkeypatch.setattr(platform, 'python_version', lambda: '3.12.8')
    monkeypatch.setattr(platform, 'python_implementation', lambda: 'CPython')
    monkeypatch.setitem(sys.modules, 'shapely', fake_module('shapely', '2.0.6'))
    monkeypatch.setitem(sys.modules, 'pyproj', fake_module('pyproj', '3.7.0'))


@pytest.fixture(autouse=True)
def home(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> Path:
    """The folder of the package, in a profile of the test."""
    folder = tmp_path / 'profile' / 'python' / 'dependencies' / 'topology_rules'
    monkeypatch.setattr(install, 'folder', lambda: folder)
    return folder


def package_modules() -> list[str]:
    return [
        name
        for name in sys.modules
        if name == install.MODULE or name.startswith(f'{install.MODULE}.')
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
        if tag == install.TAG:
            found.append(message)

    log = QgsApplication.messageLog()
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
    QgsApplication.taskManager().addTask(task)
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
    parser.read(install.METADATA, encoding='utf-8')
    assert install.wanted() == parser['general']['version']


def test_the_module_fetches_without_pip_or_other_programs():
    tree = ast.parse(Path(install.__file__).read_text(encoding='utf-8'))
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


# The wheel that fits


def mac(monkeypatch: pytest.MonkeyPatch, machine: str, version: str) -> None:
    monkeypatch.setattr(platform, 'system', lambda: 'Darwin')
    monkeypatch.setattr(platform, 'machine', lambda: machine)
    monkeypatch.setattr(
        platform, 'mac_ver', lambda: (version, ('', '', ''), '')
    )
    # What a universal2 build of Python says on Apple silicon
    monkeypatch.setattr(sysconfig, 'get_platform', lambda: 'macosx-15.7-x86_64')


def windows(monkeypatch: pytest.MonkeyPatch, build: str) -> None:
    monkeypatch.setattr(platform, 'system', lambda: 'Windows')
    monkeypatch.setattr(platform, 'machine', lambda: 'AMD64')
    monkeypatch.setattr(sysconfig, 'get_platform', lambda: build)


def listed() -> dict:
    files = [
        release_file(tag, make_wheel(tag=tag), f'https://files.invalid/{kind}')
        for kind, tag in WHEELS.items()
    ]
    return answer(files)


@pytest.mark.parametrize(
    ('system', 'expected'),
    [
        ('windows', 'windows'),
        ('mac_arm', 'mac_arm'),
        ('mac_intel', 'mac_intel'),
        ('linux', 'linux'),
    ],
)
def test_the_wheel_of_the_platform_is_chosen(
    monkeypatch: pytest.MonkeyPatch, system: str, expected: str
):
    if system == 'windows':
        windows(monkeypatch, 'win-amd64')
    elif system == 'mac_arm':
        mac(monkeypatch, 'arm64', '15.7.1')
    elif system == 'mac_intel':
        mac(monkeypatch, 'x86_64', '13.6')
    release = install.pick(listed(), VERSION)
    assert release.filename == wheel_name(WHEELS[expected])
    assert release.url == f'https://files.invalid/{expected}'


def test_a_release_holds_what_the_consent_shows(home: Path):
    data = listed()
    release = install.pick(data, VERSION)
    linux = next(f for f in data['urls'] if f['filename'] == release.filename)
    assert release.version == VERSION
    assert release.size == linux['size']
    assert release.sha256 == linux['digests']['sha256']
    assert release.target == home / VERSION


def test_an_older_macos_than_the_wheel_has_none(
    monkeypatch: pytest.MonkeyPatch,
):
    mac(monkeypatch, 'x86_64', '10.11')
    with pytest.raises(install.Failed, match='no wheel'):
        install.pick(listed(), VERSION)


def test_a_wheel_for_another_python_is_not_chosen():
    tag = 'cp313-cp313-manylinux_2_17_x86_64'
    data = answer([release_file(tag, b'x', 'https://files.invalid/313')])
    with pytest.raises(install.Failed, match='no wheel'):
        install.pick(data, VERSION)


def test_another_version_from_pypi_is_refused():
    data = listed()
    data['info']['version'] = '0.0.1'
    with pytest.raises(install.Failed, match=r'0\.0\.1 for '):
        install.pick(data, VERSION)


def test_a_file_without_a_sha256_is_refused():
    data = listed()
    for file in data['urls']:
        file['digests'] = {}
    with pytest.raises(install.Failed, match='SHA-256'):
        install.pick(data, VERSION)


# Why this QGIS cannot use the package


def test_this_qgis_can_use_it():
    assert install.unsupported() is None


def thirty_two_bits(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(sys, 'maxsize', 2**31 - 1)


@pytest.mark.parametrize(
    ('patch', 'reason'),
    [
        (
            lambda m: m.setattr(platform, 'python_version', lambda: '3.9.18'),
            'Needs Python 3.10 or newer, QGIS has Python 3.9.18',
        ),
        (
            lambda m: m.setattr(
                platform, 'python_implementation', lambda: 'PyPy'
            ),
            'Needs CPython, QGIS has PyPy',
        ),
        (
            lambda m: windows(m, 'win32'),
            'No wheel for Windows win32',
        ),
        (
            lambda m: windows(m, 'win-arm64'),
            'No wheel for Windows win-arm64',
        ),
        (
            lambda m: mac(m, 'i386', '10.15'),
            'No wheel for macOS i386',
        ),
        (
            lambda m: m.setattr(platform, 'machine', lambda: 'aarch64'),
            'No wheel for Linux aarch64',
        ),
        (thirty_two_bits, 'No wheel for 32-bit Python'),
        (
            lambda m: m.setattr(platform, 'libc_ver', lambda: ('', '')),
            'Needs glibc 2.17 or newer, Linux has no glibc',
        ),
        (
            lambda m: m.setattr(
                platform, 'libc_ver', lambda: ('glibc', '2.12')
            ),
            'Needs glibc 2.17 or newer, Linux has glibc 2.12',
        ),
        (
            lambda m: m.setattr(platform, 'system', lambda: 'FreeBSD'),
            'No wheel for FreeBSD x86_64',
        ),
        (
            lambda m: m.setitem(sys.modules, 'shapely', None),
            'Needs shapely 2 or newer, which does not load',
        ),
        (
            lambda m: m.setitem(
                sys.modules, 'shapely', fake_module('shapely', '1.8.5')
            ),
            'Needs shapely 2 or newer, QGIS has shapely 1.8.5',
        ),
        (
            lambda m: m.setitem(sys.modules, 'pyproj', None),
            'Needs pyproj 3.4 or newer, which does not load',
        ),
        (
            lambda m: m.setitem(
                sys.modules, 'pyproj', fake_module('pyproj', '3.3.1')
            ),
            'Needs pyproj 3.4 or newer, QGIS has pyproj 3.3.1',
        ),
    ],
)
def test_why_this_qgis_cannot_use_it_is_said(
    monkeypatch: pytest.MonkeyPatch,
    patch: c.Callable[[pytest.MonkeyPatch], None],
    reason: str,
):
    patch(monkeypatch)
    found = install.unsupported()
    assert found is not None
    assert found.startswith(reason)


def test_a_qgis_that_cannot_use_it_asks_pypi_nothing(
    pypi: Index, monkeypatch: pytest.MonkeyPatch, home: Path
):
    monkeypatch.setattr(platform, 'machine', lambda: 'aarch64')
    outcome = run(install.ReleaseTask())
    assert outcome.failed == ['No wheel for Linux aarch64']
    assert outcome.found == []
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
    assert error == (
        f'SHA-256 {tampered} does not match PyPI {sha256(content)}'
    )
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
    url = pypi.serve('/files/wheel.whl', content)
    pypi.release(answer([release_file(LINUX, content, url)]))
    outcome = run(install.InstallTask(find()))
    assert outcome.failed == [f'Refused {name}: outside of the folder']
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
    loaded = fake_module('topology_checker', '0.0.1')
    loaded.__file__ = str(old / 'topology_checker' / '__init__.py')
    monkeypatch.setitem(sys.modules, 'topology_checker', loaded)
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
        'Python 3.12.8, shapely 2.0.6, pyproj 3.7.0',
        'Platform Linux x86_64 glibc 2.35',
        f'Fetching {pypi.url}/pypi/topology-checker/{VERSION}/json',
        f'Found {name} ({size}), SHA-256 {digest}',
        f'Downloading {name} ({size})',
        f'From {pypi.url}/files/{name}',
        f'SHA-256 {digest} matches PyPI',
        f'Unpacked 4 files into {home / VERSION}',
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
    loaded = fake_module('topology_checker', '0.0.1')
    loaded.__file__ = str(old / 'topology_checker' / '__init__.py')
    monkeypatch.setitem(sys.modules, 'topology_checker', loaded)
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
    monkeypatch.setattr(install, 'wanted', lambda: '0.10.0')
    assert install.installed() == '0.2.1'
    assert install.outdated()


def test_the_newest_install_is_used(
    home: Path, monkeypatch: pytest.MonkeyPatch
):
    version_folder(home, '0.2.0')
    version_folder(home, '0.10.0')
    monkeypatch.setattr(install, 'wanted', lambda: '0.10.0')
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

    def locked(path: t.Any, *args: t.Any, **kwargs: t.Any) -> None:
        # Windows keeps a library that is loaded
        if str(path).endswith('.pyd'):
            raise PermissionError(13, 'Access is denied', str(path))
        unlink(path, *args, **kwargs)

    monkeypatch.setattr(os, 'unlink', locked)
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
    assert install.command() == (
        f'python3 -m pip install --no-deps --target {home / VERSION} '
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
    assert install.command() == (
        f"PYTHONHOME='{contents / 'Frameworks'}' '{python}' -m pip install "
        f'--no-deps --target {home / VERSION} topology-checker=={VERSION}'
    )
    assert install.terminal() == 'Terminal'


def test_the_command_quotes_a_folder_with_spaces(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
):
    folder = tmp_path / 'Application Support' / 'topology_rules'
    monkeypatch.setattr(install, 'folder', lambda: folder)
    assert f"--target '{folder / VERSION}' " in install.command()
