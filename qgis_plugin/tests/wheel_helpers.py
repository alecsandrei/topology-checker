"""Wheels, PyPI answers and platforms for the tests of the package install."""

from __future__ import annotations

import hashlib
import io
import platform
import sys
import sysconfig
import types
import typing as t
import zipfile

import pytest

from topology_rules.core import folders

VERSION = folders.wanted()
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


def listed() -> dict:
    """A release with a wheel for each platform, at URLs named after it."""
    files = [
        release_file(tag, make_wheel(tag=tag), f'https://files.invalid/{kind}')
        for kind, tag in WHEELS.items()
    ]
    return answer(files)


# The platforms


def fake_module(name: str, version: str) -> types.ModuleType:
    module = types.ModuleType(name)
    module.__version__ = version  # type: ignore[attr-defined]
    return module


def pretend_linux(monkeypatch: pytest.MonkeyPatch) -> None:
    """Linux x86_64 with glibc 2.35 and Python 3.12.8, whatever the machine
    of the tests. shapely and pyproj do not import, as the package does not
    need them."""
    monkeypatch.setattr(platform, 'system', lambda: 'Linux')
    monkeypatch.setattr(platform, 'machine', lambda: 'x86_64')
    monkeypatch.setattr(platform, 'libc_ver', lambda: ('glibc', '2.35'))
    monkeypatch.setattr(platform, 'python_version', lambda: '3.12.8')
    monkeypatch.setattr(platform, 'python_implementation', lambda: 'CPython')
    for name in ('shapely', 'pyproj'):
        monkeypatch.setitem(sys.modules, name, None)


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
