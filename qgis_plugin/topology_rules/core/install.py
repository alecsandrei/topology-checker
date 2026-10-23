"""The Python package topology-checker, installed from PyPI into the profile
of QGIS, so that the checks run in QGIS rather than in the executable.

There is no pip: the wheel of this platform is fetched with the network
settings of QGIS, checked against the SHA-256 that PyPI gives, and only the
package is unpacked, as its dependencies are those of QGIS. Each version has a
folder of its own, as Windows locks a library that is loaded.
"""

from __future__ import annotations

import collections.abc as c
import configparser
import dataclasses
import datetime
import hashlib
import importlib
import importlib.metadata
import io
import json
import os
import platform
import re
import shlex
import shutil
import sys
import sysconfig
import tempfile
import threading
import zipfile
from pathlib import Path

from qgis.core import (
    Qgis,
    QgsApplication,
    QgsBlockingNetworkRequest,
    QgsFeedback,
    QgsMessageLog,
    QgsTask,
)
from qgis.PyQt.QtCore import QCoreApplication, QUrl, pyqtSignal
from qgis.PyQt.QtNetwork import QNetworkRequest

from ..utils import tr

# The index of the releases; the tests point it to a server of their own
PYPI = 'https://pypi.org/pypi'
PACKAGE = 'topology-checker'
MODULE = 'topology_checker'
# The tab of the Log Messages panel
TAG = 'Topology Rules'
LOG = 'install.log'
METADATA = Path(__file__).parents[1] / 'metadata.txt'
# What the package needs, which QGIS brings
PYTHON = (3, 10)
NEEDS = {'shapely': (2,), 'pyproj': (3, 4)}
GLIBC = (2, 17)
# The names of a version folder, and of the folder that an install fills
# before it becomes one
VERSION = re.compile(r'\d+(\.\d+)*')
UNPACKING = re.compile(r'\.\d+(\.\d+)*-.+')
# The top folders of a wheel that are installed: the package, its
# dist-info, and the libraries that a repaired Linux wheel links to
KEPT = re.compile(rf'{MODULE}|{MODULE}-[^/]+\.dist-info|{MODULE}\.libs')

_writing = threading.Lock()


class Unsupported(Exception):
    """Why this QGIS cannot load the package."""


class Failed(Exception):
    """Why a step of the install failed, in words."""


@dataclasses.dataclass(frozen=True)
class Release:
    """The wheel of a release on PyPI that fits this QGIS."""

    version: str
    filename: str
    url: str
    size: int
    sha256: str

    @property
    def target(self) -> Path:
        """The folder that the release is installed into."""
        return folder() / self.version


# Versions


def numbers(text: str) -> tuple[int, ...]:
    """The numbers that a version starts with: '2.0.6' as (2, 0, 6)."""
    found = VERSION.match(text)
    if found is None:
        return ()
    return tuple(int(part) for part in found.group().split('.'))


def folder() -> Path:
    """Where the package is installed: in the profile, which is writable and
    kept when the plugin is upgraded. Holds a folder for each version and the
    install log."""
    profile = Path(QgsApplication.qgisSettingsDirPath())
    return profile / 'python' / 'dependencies' / 'topology_rules'


def wanted() -> str:
    """The version to install, which is that of the plugin."""
    parser = configparser.ConfigParser(interpolation=None)
    parser.read(METADATA, encoding='utf-8')
    return parser['general']['version'].strip()


def versions() -> list[Path]:
    """The version folders in folder(), newest first."""
    try:
        entries = list(folder().iterdir())
    except OSError:
        return []
    found = [
        path
        for path in entries
        if path.is_dir() and VERSION.fullmatch(path.name)
    ]
    return sorted(found, key=lambda path: numbers(path.name), reverse=True)


def current() -> Path | None:
    """The version folder that the plugin loads: the newest one that loads
    here and is not newer than the plugin."""
    limit = numbers(wanted())
    for path in versions():
        if numbers(path.name) <= limit and problem(path) is None:
            return path
    return None


def installed() -> str | None:
    """The version of the package in folder() that the plugin loads."""
    path = current()
    return None if path is None else path.name


def outdated() -> bool:
    """Whether the installed package is older than the plugin. It is not
    updated on its own: the update is a new install."""
    version = installed()
    return version is not None and numbers(version) < numbers(wanted())


# This QGIS


def dependency(name: str, least: tuple[int, ...]) -> str:
    """The version of a module of QGIS that the package needs. Raises
    Unsupported when it does not import."""
    needed = '.'.join(map(str, least))
    try:
        module = importlib.import_module(name)
    except Exception as error:
        raise Unsupported(
            tr(
                'Needs {name} {needed} or newer, which does not load: {error}'
            ).format(name=name, needed=needed, error=error)
        ) from error
    return str(getattr(module, '__version__', ''))


def platform_tags() -> list[str]:
    """The platform tags of the wheels that load here, best first. Raises
    Unsupported where no wheel of the package loads."""
    system = platform.system()
    machine = platform.machine()
    if system == 'Windows':
        # The build of Python, as an x64 QGIS also runs on ARM
        build = sysconfig.get_platform()
        if build == 'win-amd64':
            return ['win_amd64']
        raise Unsupported(
            tr('No wheel for Windows {build}').format(build=build)
        )
    if system == 'Darwin':
        # The machine, as sysconfig says x86_64 for universal2 on arm64
        if machine not in ('arm64', 'x86_64'):
            raise Unsupported(
                tr('No wheel for macOS {machine}').format(machine=machine)
            )
        return mac_tags(machine, numbers(platform.mac_ver()[0]) or (11, 0))
    if system == 'Linux':
        if machine != 'x86_64':
            raise Unsupported(
                tr('No wheel for Linux {machine}').format(machine=machine)
            )
        if sys.maxsize <= 2**32:
            raise Unsupported(tr('No wheel for 32-bit Python'))
        libc, version = platform.libc_ver()
        glibc = numbers(version) if libc == 'glibc' else ()
        if glibc < GLIBC:
            found = f'{libc} {version}'.strip() or tr('no glibc')
            raise Unsupported(
                tr('Needs glibc 2.17 or newer, Linux has {found}').format(
                    found=found
                )
            )
        return linux_tags(glibc)
    raise Unsupported(
        tr('No wheel for {system} {machine}').format(
            system=system or tr('this system'), machine=machine
        )
    )


def mac_tags(machine: str, version: tuple[int, ...]) -> list[str]:
    """Wheels for this macOS or an older one. From macOS 11 on only the
    major version is in the tag."""
    major, minor = (*version, 0)[:2]
    tags: list[str] = []
    for number in range(major, 10, -1):
        tags += [
            f'macosx_{number}_0_{machine}',
            f'macosx_{number}_0_universal2',
        ]
    if machine == 'x86_64':
        newest = minor if major == 10 else 16
        for number in range(newest, 3, -1):
            tags += [
                f'macosx_10_{number}_x86_64',
                f'macosx_10_{number}_universal2',
            ]
    return tags


def linux_tags(glibc: tuple[int, ...]) -> list[str]:
    """manylinux wheels for this glibc or an older one, with their old
    names."""
    old = {17: 'manylinux2014', 12: 'manylinux2010', 5: 'manylinux1'}
    tags: list[str] = []
    for minor in range(glibc[1], 4, -1):
        tags.append(f'manylinux_2_{minor}_x86_64')
        if minor in old:
            tags.append(f'{old[minor]}_x86_64')
    return tags


def python_fits(python: str, abi: str) -> bool:
    """Whether this Python loads a wheel of the Python and ABI tags."""
    version = numbers(platform.python_version())[:2]
    cpython = platform.python_implementation() == 'CPython'
    found = re.fullmatch(r'cp(\d)(\d+)', python)
    if abi == 'abi3':
        least = (int(found[1]), int(found[2])) if found else None
        return cpython and least is not None and least <= version
    if abi == 'none':
        return python in ('py3', 'py{}{}'.format(*version))
    exact = 'cp{}{}'.format(*version)
    return cpython and python == exact and abi == exact


def rank(tag: str | None) -> int | None:
    """How well a wheel of the tag, such as 'cp310-abi3-win_amd64', fits
    here: lower is better, and None when it does not load. A part of the tag
    may list several with dots."""
    parts = (tag or '').split('-')
    if len(parts) != 3:
        return None
    try:
        accepted = platform_tags()
    except Unsupported:
        return None
    found = [
        accepted.index(plat)
        for python in parts[0].split('.')
        for abi in parts[1].split('.')
        for plat in parts[2].split('.')
        if plat in accepted and python_fits(python, abi)
    ]
    return min(found, default=None)


def wheel_tag(filename: str) -> str | None:
    """The tag of a wheel from its file name, without the build number."""
    if not filename.endswith('.whl'):
        return None
    parts = filename[: -len('.whl')].split('-')
    return '-'.join(parts[-3:]) if len(parts) >= 5 else None


def check() -> None:
    """Raises Unsupported when this QGIS cannot load the package."""
    python = platform.python_version()
    implementation = platform.python_implementation()
    if implementation != 'CPython':
        raise Unsupported(
            tr('Needs CPython, QGIS has {implementation}').format(
                implementation=implementation
            )
        )
    if numbers(python)[:2] < PYTHON:
        raise Unsupported(
            tr('Needs Python 3.10 or newer, QGIS has Python {python}').format(
                python=python
            )
        )
    platform_tags()
    for name, least in NEEDS.items():
        version = dependency(name, least)
        if numbers(version) < least:
            raise Unsupported(
                tr(
                    'Needs {name} {needed} or newer, QGIS has {name} {version}'
                ).format(
                    name=name,
                    needed='.'.join(map(str, least)),
                    version=version or tr('of no version'),
                )
            )


def unsupported() -> str | None:
    """Why this QGIS cannot use the package, or None when it can. Where it
    cannot, the plugin keeps to the executable."""
    try:
        check()
    except Unsupported as error:
        return str(error)
    return None


def dependencies() -> dict[str, str | None]:
    """The versions of the modules of QGIS that the package uses, such as
    {'shapely': '2.0.6', 'pyproj': '3.7.0'}, with None for one that does not
    load. The install adds none of them."""
    found: dict[str, str | None] = {}
    for name, least in NEEDS.items():
        try:
            found[name] = dependency(name, least)
        except Unsupported:
            found[name] = None
    return found


def environment(found: dict[str, str | None] | None = None) -> str:
    """The versions that decide whether the package loads, for the log."""
    found = dependencies() if found is None else found
    parts = [f'Python {platform.python_version()}']
    for name, version in found.items():
        parts.append(f'{name} {version or tr("missing")}')
    return ', '.join(parts)


def nothing_else(found: dict[str, str | None]) -> str:
    """That the install adds no other package, for the log: 'No other
    packages installed. Uses QGIS's shapely 2.0.6 and pyproj 3.7.0.'"""
    return tr(
        "No other packages installed. Uses QGIS's shapely {shapely} and "
        'pyproj {pyproj}.'
    ).format(shapely=found.get('shapely'), pyproj=found.get('pyproj'))


def described() -> str:
    """This platform in words, for the log."""
    system = platform.system()
    machine = platform.machine()
    if system == 'Darwin':
        return f'macOS {platform.mac_ver()[0]} {machine}'
    if system == 'Windows':
        return f'Windows {sysconfig.get_platform()}'
    if system == 'Linux':
        return ' '.join(['Linux', machine, *platform.libc_ver()]).strip()
    return f'{system} {machine}'


# The folders


def inside(path: str | Path, parent: Path) -> bool:
    return Path(os.path.abspath(path)).is_relative_to(os.path.abspath(parent))


def problem(path: Path, version: str | None = None) -> str | None:
    """Why a version folder does not load here, or None when it does. A
    folder that a profile brought from another QGIS may be for another
    Python or platform."""
    version = version or path.name
    infos = sorted(path.glob(f'{MODULE}-*.dist-info'))
    if not infos:
        return tr('no dist-info')
    if len(infos) > 1:
        return tr('several dist-info folders')
    try:
        distribution = importlib.metadata.Distribution.at(infos[0])
        found = distribution.version
        wheel = distribution.read_text('WHEEL') or ''
    except Exception as error:
        return str(error)
    if found != version:
        return tr('holds {package} {found}').format(
            package=PACKAGE, found=found
        )
    tags = [
        line.partition(':')[2].strip()
        for line in wheel.splitlines()
        if line.startswith('Tag:')
    ]
    if not any(rank(tag) is not None for tag in tags):
        return tr('built for {tags}').format(
            tags=', '.join(tags) or tr('no platform')
        )
    if not (path / MODULE / '__init__.py').is_file():
        return tr('no {module} package').format(module=MODULE)
    return None


def stale(path: Path, keep: Path | None) -> str | None:
    """Why a folder in folder() is deleted, or None when it stays. A folder
    for another Python or a newer plugin stays, and is not loaded."""
    if UNPACKING.fullmatch(path.name):
        return tr('left by an install that did not finish')
    if not VERSION.fullmatch(path.name):
        return None
    if not any(path.glob(f'{MODULE}-*.dist-info')):
        return tr('no dist-info')
    if keep is not None and numbers(path.name) < numbers(keep.name):
        return tr('replaced by {version}').format(version=keep.name)
    return None


def delete(path: Path) -> str | None:
    """Deletes a folder, as much of it as can be, and returns why some of it
    is left, or None."""
    try:
        shutil.rmtree(path)
    except FileNotFoundError:
        return None
    except OSError as error:
        shutil.rmtree(path, ignore_errors=True)
        return str(error)
    return None


def uninstall(path: Path) -> str | None:
    """Deletes a version folder, its dist-info first, so that what a locked
    library keeps of it is not taken as installed. Returns why some of it is
    left, or None."""
    for info in path.glob(f'{MODULE}-*.dist-info'):
        delete(info)
    return delete(path)


def origin() -> Path | None:
    """The file of the topology_checker that is imported, if any."""
    module = sys.modules.get(MODULE)
    file = getattr(module, '__file__', None)
    return Path(file) if file else None


def unlisted(parent: Path) -> None:
    """Takes the folders under parent off sys.path."""
    sys.path[:] = [entry for entry in sys.path if not inside(entry, parent)]


def use(path: Path) -> None:
    """Puts a version folder first on sys.path, so that the next import of
    topology_checker finds it, and drops the other versions from it."""
    unlisted(folder())
    sys.path.insert(0, str(path))
    importlib.invalidate_caches()


def purge(keep: Path | None) -> None:
    """Deletes the folders that are not used any more. The folder of a
    package that is loaded stays, as Windows locks its library."""
    loaded = origin()
    try:
        entries = sorted(folder().iterdir())
    except OSError:
        return
    for path in entries:
        if not path.is_dir() or path == keep:
            continue
        if loaded is not None and inside(loaded, path):
            continue
        reason = stale(path, keep)
        if reason is None:
            continue
        error = uninstall(path)
        if error is None:
            log(tr('Removed {path}: {reason}').format(path=path, reason=reason))
        else:
            log(
                tr('Could not remove {path}: {error}').format(
                    path=path, error=error
                ),
                Qgis.MessageLevel.Warning,
            )


def activate() -> None:
    """Deletes the versions that are not used, and puts the one that is
    first on sys.path. Runs at plugin load, before anything imports
    topology_checker; it never raises, as the plugin must load."""
    try:
        if not QgsApplication.qgisSettingsDirPath():
            return
        path = current()
        purge(path)
        if path is not None:
            use(path)
    except Exception as error:
        log(
            tr('Could not activate {package}: {error}').format(
                package=PACKAGE, error=error
            ),
            Qgis.MessageLevel.Warning,
        )


def restart_needed() -> bool:
    """Whether QGIS must restart to load what folder() holds: a package
    that is loaded stays loaded, from where it came."""
    loaded = origin()
    if loaded is None:
        return False
    path = current()
    if path is not None:
        return not inside(loaded, path)
    return inside(loaded, folder())


def remove() -> None:
    """Deletes folder() and takes it off sys.path. A package that is loaded
    stays loaded until QGIS restarts, and on Windows so does its library on
    disk; the next start deletes it."""
    home = folder()
    unlisted(home)
    importlib.invalidate_caches()
    if not home.exists():
        return
    for path in versions():
        uninstall(path)
    error = delete(home)
    if error is None:
        log(tr('Removed {folder}').format(folder=home), file=False)
    else:
        log(
            tr('Could not remove all of {folder}: {error}').format(
                folder=home, error=error
            ),
            Qgis.MessageLevel.Warning,
        )


# Logs and commands


def log(
    text: str,
    level: Qgis.MessageLevel = Qgis.MessageLevel.Info,
    file: bool = True,
) -> None:
    """Writes a step to the Log Messages panel and to the install log."""
    notify = level != Qgis.MessageLevel.Info
    QgsMessageLog.logMessage(text, TAG, level, notify)
    if not file:
        return
    stamp = datetime.datetime.now().strftime('%Y-%m-%d %H:%M:%S')
    try:
        home = folder()
        home.mkdir(parents=True, exist_ok=True)
        with _writing, (home / LOG).open('a', encoding='utf-8') as handle:
            handle.write(f'{stamp} {text}\n')
    except OSError as error:
        QgsMessageLog.logMessage(
            tr('Cannot write {log}: {error}').format(log=LOG, error=error),
            TAG,
            Qgis.MessageLevel.Warning,
        )


def size_text(size: int) -> str:
    """'3.1 MB', in kilobytes below a tenth of a megabyte."""
    if size >= 100_000:
        return f'{size / 1_000_000:.1f} MB'
    return f'{max(1, round(size / 1000))} KB'


def mac_python() -> str:
    """The Python of QGIS on macOS: the script of the app that sets its
    home, or else the Python itself with its home."""
    app = Path(QCoreApplication.applicationDirPath())
    if (app / 'python').is_file():
        return shlex.quote(str(app / 'python'))
    name = 'python{}.{}'.format(*numbers(platform.python_version())[:2])
    path = app / name
    if not path.is_file():
        return 'python3'
    if '.app/Contents/' in path.as_posix():
        home = shlex.quote(sys.base_prefix)
        return f'PYTHONHOME={home} {shlex.quote(str(path))}'
    return shlex.quote(str(path))


def command(version: str | None = None) -> str:
    """The pip command that installs the same as the plugin does, for the
    shell that terminal() names."""
    version = version or wanted()
    target = str(folder() / version)
    system = platform.system()
    quoted = f'"{target}"' if system == 'Windows' else shlex.quote(target)
    pip = f'-m pip install --no-deps --target {quoted} {PACKAGE}=={version}'
    if system == 'Windows':
        return f'python {pip}'
    if system == 'Darwin':
        return f'{mac_python()} {pip}'
    return f'python3 {pip}'


def terminal() -> str:
    """Where command() is run."""
    if platform.system() == 'Windows':
        return tr('OSGeo4W Shell')
    return tr('Terminal')


# PyPI


def pick(data: object, version: str) -> Release:
    """The wheel of the release in a PyPI answer that fits here best. Raises
    Failed when there is none."""
    if not isinstance(data, dict) or not isinstance(data.get('urls'), list):
        raise Failed(tr('PyPI sent no files'))
    files: list[object] = data['urls']
    info = data.get('info')
    found = info.get('version') if isinstance(info, dict) else None
    if found != version:
        raise Failed(
            tr('PyPI sent {package} {found} for {version}').format(
                package=PACKAGE, found=found, version=version
            )
        )
    ranked = [
        (score, file)
        for file in files
        if isinstance(file, dict)
        for score in [rank(wheel_tag(str(file.get('filename', ''))))]
        if score is not None
    ]
    if not ranked:
        raise Failed(
            tr(
                'PyPI has no wheel of {package} {version} for {platform}'
            ).format(package=PACKAGE, version=version, platform=described())
        )
    file = min(ranked, key=lambda pair: pair[0])[1]
    filename = str(file['filename'])
    digests = file.get('digests')
    sha256 = str(digests.get('sha256', '')) if isinstance(digests, dict) else ''
    if not re.fullmatch(r'[0-9a-f]{64}', sha256.lower()):
        raise Failed(
            tr('PyPI sent no SHA-256 for {file}').format(file=filename)
        )
    url = str(file.get('url', ''))
    if not url:
        raise Failed(tr('PyPI sent no URL for {file}').format(file=filename))
    try:
        size = int(file.get('size') or 0)
    except (TypeError, ValueError):
        size = 0
    return Release(version, filename, url, size, sha256.lower())


def escapes(name: str) -> bool:
    """Whether a member of a zip file would land outside of the folder that
    it is unpacked into."""
    parts = re.split(r'[\\/]', name)
    return (
        name.startswith(('/', '\\'))
        or re.match(r'[A-Za-z]:', name) is not None
        or '..' in parts
    )


def unpack(
    content: bytes, version: str, target: Path, say: c.Callable[[str], None]
) -> int:
    """Unpacks the package of a wheel into the target folder, by way of a
    folder next to it, and returns the number of files. Raises Failed for a
    wheel with a file that would land outside of it."""
    try:
        wheel = zipfile.ZipFile(io.BytesIO(content))
    except zipfile.BadZipFile as error:
        raise Failed(tr('Not a wheel: {error}').format(error=error)) from error
    with wheel:
        members = wheel.infolist()
        for member in members:
            if escapes(member.filename):
                raise Failed(
                    tr('Refused {name}: outside of the folder').format(
                        name=member.filename
                    )
                )
        kept = [m for m in members if KEPT.fullmatch(top(m.filename))]
        for name in sorted(
            {top(m.filename) for m in members} - {top(m.filename) for m in kept}
        ):
            say(tr('Left out {name}').format(name=name))
        names = {member.filename for member in kept}
        if f'{MODULE}/__init__.py' not in names:
            raise Failed(
                tr('The wheel has no {module} package').format(module=MODULE)
            )
        home = target.parent
        home.mkdir(parents=True, exist_ok=True)
        unpacking = Path(tempfile.mkdtemp(prefix=f'.{version}-', dir=home))
        try:
            for member in kept:
                wheel.extract(member, unpacking)
            reason = problem(unpacking, version)
            if reason is not None:
                raise Failed(
                    tr('The wheel does not load here: {reason}').format(
                        reason=reason
                    )
                )
            if target.exists():
                error = delete(target)
                if error is not None:
                    raise Failed(
                        tr(
                            'Cannot replace {folder}: {error}. Restart QGIS '
                            'and install again'
                        ).format(folder=target, error=error)
                    )
            unpacking.rename(target)
        except BaseException:
            delete(unpacking)
            raise
    return sum(1 for member in kept if not member.is_dir())


def top(name: str) -> str:
    return name.split('/', 1)[0]


class Task(QgsTask):
    """A step of the install that logs what it does, live and to the logs,
    and says why it failed."""

    logged = pyqtSignal(str)
    failed = pyqtSignal(str)

    def __init__(self, description: str):
        super().__init__(description)
        self.feedback = QgsFeedback()
        self.error = ''

    def say(
        self, text: str, level: Qgis.MessageLevel = Qgis.MessageLevel.Info
    ) -> None:
        log(text, level)
        self.logged.emit(text)

    def cancel(self) -> None:
        # The feedback aborts a download that is under way
        self.feedback.cancel()
        super().cancel()

    def get(self, url: str, missing: str = '') -> bytes:
        """The body of the URL, fetched with the proxy, SSL and
        authentication settings of QGIS. Raises Failed with missing, when
        given, for a URL that is not found."""
        request = QgsBlockingNetworkRequest()
        request.downloadProgress.connect(
            lambda done, total: (
                self.setProgress(100 * done / total) if total > 0 else None
            )
        )
        code = request.get(QNetworkRequest(QUrl(url)), True, self.feedback)
        if self.isCanceled():
            raise Failed(tr('Cancelled'))
        if code == QgsBlockingNetworkRequest.ErrorCode.NoError:
            return bytes(request.reply().content())
        reply = request.reply()
        status = reply.attribute(
            QNetworkRequest.Attribute.HttpStatusCodeAttribute
        )
        if status == 404 and missing:
            raise Failed(missing)
        if isinstance(status, int):
            reason = reply.attribute(
                QNetworkRequest.Attribute.HttpReasonPhraseAttribute
            )
            error = f'HTTP {status} {reason or ""}'.strip()
        else:
            error = request.errorMessage()
        raise Failed(
            tr('Cannot fetch {url}: {error}').format(url=url, error=error)
        )

    def work(self) -> None:
        raise NotImplementedError

    def succeed(self) -> None:
        raise NotImplementedError

    def run(self) -> bool:
        # Errors are kept and not raised, as QGIS 4 does not call back a
        # task whose run raised
        try:
            self.work()
        except Exception as error:
            if self.isCanceled():
                self.error = tr('Cancelled')
            elif isinstance(error, Failed):
                self.error = str(error)
            else:
                self.error = f'{type(error).__name__}: {error}'
            self.say(self.error, Qgis.MessageLevel.Critical)
            return False
        return True

    def finished(self, result: bool) -> None:
        if result:
            self.succeed()
            return
        if not self.error:
            # Cancelled before it ran
            self.error = tr('Cancelled')
            self.say(self.error, Qgis.MessageLevel.Critical)
        self.failed.emit(self.error)


class ReleaseTask(Task):
    """Finds the wheel of a release for this platform on PyPI, without
    downloading it, so that its file, size, source and SHA-256 can be shown
    before anything is installed."""

    found = pyqtSignal(object)

    def __init__(self, version: str | None = None):
        version = version or wanted()
        super().__init__(
            tr('Finding {package} {version}').format(
                package=PACKAGE, version=version
            )
        )
        self.version = version
        self.url = f'{PYPI}/{PACKAGE}/{version}/json'
        self.release: Release | None = None
        # Asked here, as the modules are imported on the main thread
        self.reason = unsupported()
        self.dependencies = dependencies()

    def work(self) -> None:
        self.say(environment(self.dependencies))
        self.say(tr('Platform {platform}').format(platform=described()))
        if self.reason is not None:
            raise Failed(self.reason)
        self.say(nothing_else(self.dependencies))
        self.say(tr('Fetching {url}').format(url=self.url))
        content = self.get(
            self.url,
            tr('PyPI has no {package} {version}').format(
                package=PACKAGE, version=self.version
            ),
        )
        try:
            data = json.loads(content)
        except ValueError as error:
            raise Failed(
                tr('PyPI sent no JSON: {error}').format(error=error)
            ) from error
        release = pick(data, self.version)
        self.say(
            tr('Found {file} ({size}), SHA-256 {sha256}').format(
                file=release.filename,
                size=size_text(release.size),
                sha256=release.sha256,
            )
        )
        if self.isCanceled():
            raise Failed(tr('Cancelled'))
        self.release = release

    def succeed(self) -> None:
        self.found.emit(self.release)


class InstallTask(Task):
    """Downloads the wheel of a release, checks its SHA-256, unpacks the
    package into its version folder and loads it. A package that is already
    loaded is replaced at the next start."""

    done = pyqtSignal(str)

    def __init__(self, release: Release):
        super().__init__(
            tr('Installing {package} {version}').format(
                package=PACKAGE, version=release.version
            )
        )
        self.release = release

    def work(self) -> None:
        release = self.release
        self.say(
            tr('Downloading {file} ({size})').format(
                file=release.filename, size=size_text(release.size)
            )
        )
        self.say(tr('From {url}').format(url=release.url))
        content = self.get(release.url)
        digest = hashlib.sha256(content).hexdigest()
        if digest != release.sha256:
            raise Failed(
                tr('SHA-256 {digest} does not match PyPI {expected}').format(
                    digest=digest, expected=release.sha256
                )
            )
        self.say(tr('SHA-256 {digest} matches PyPI').format(digest=digest))
        if self.isCanceled():
            raise Failed(tr('Cancelled'))
        count = unpack(content, release.version, release.target, self.say)
        self.say(
            tr('Unpacked {count} files into {folder}').format(
                count=count, folder=release.target
            )
        )

    def succeed(self) -> None:
        version = self.release.version
        target = self.release.target
        use(target)
        loaded = origin()
        if loaded is not None and not inside(loaded, target):
            self.say(
                tr(
                    'Restart QGIS to load {package} {version}, as the one '
                    'in {path} is loaded'
                ).format(package=PACKAGE, version=version, path=loaded.parent)
            )
            self.done.emit(version)
            return
        try:
            importlib.import_module(MODULE)
        except Exception as error:
            for name in [
                name
                for name in sys.modules
                if name == MODULE or name.startswith(f'{MODULE}.')
            ]:
                del sys.modules[name]
            unlisted(target)
            uninstall(target)
            self.error = tr(
                '{package} {version} does not load: {error}'
            ).format(package=PACKAGE, version=version, error=error)
            self.say(self.error, Qgis.MessageLevel.Critical)
            self.failed.emit(self.error)
            return
        self.say(
            tr('Loaded {package} {version}').format(
                package=PACKAGE, version=version
            )
        )
        self.done.emit(version)
