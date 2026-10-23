"""Which wheel of topology-checker loads in this Python, and unpacking one
safely. Nothing here asks QGIS but for translations, so the tests can give it
any platform.
"""

from __future__ import annotations

import collections.abc as c
import dataclasses
import importlib
import importlib.metadata
import io
import platform
import re
import shutil
import sys
import sysconfig
import tempfile
import zipfile
from pathlib import Path

from ..utils import tr

PACKAGE = 'topology-checker'
MODULE = 'topology_checker'
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


def numbers(text: str) -> tuple[int, ...]:
    """The numbers that a version starts with: '2.0.6' as (2, 0, 6)."""
    found = VERSION.match(text)
    if found is None:
        return ()
    return tuple(int(part) for part in found.group().split('.'))


# This Python


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


# The tags of the wheels


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


# Unpacking


def top(name: str) -> str:
    """The top folder of a member of a zip file, or its name."""
    return name.split('/', 1)[0]


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
    content: bytes, version: str, home: Path, say: c.Callable[[str], None]
) -> tuple[Path, int]:
    """Unpacks the package of a wheel into a new folder in home, and returns
    the folder and its number of files. Raises Failed, and leaves nothing, for
    a wheel with a file that would land outside of it, or that does not load
    here."""
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
        if f'{MODULE}/__init__.py' not in {m.filename for m in kept}:
            raise Failed(
                tr('The wheel has no {module} package').format(module=MODULE)
            )
        home.mkdir(parents=True, exist_ok=True)
        unpacked = Path(tempfile.mkdtemp(prefix=f'.{version}-', dir=home))
        try:
            for member in kept:
                wheel.extract(member, unpacked)
            reason = problem(unpacked, version)
            if reason is not None:
                raise Failed(
                    tr('The wheel does not load here: {reason}').format(
                        reason=reason
                    )
                )
        except BaseException:
            shutil.rmtree(unpacked, ignore_errors=True)
            raise
    return unpacked, sum(1 for member in kept if not member.is_dir())
