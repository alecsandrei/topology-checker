"""The wheel of the package that fits this QGIS, and why a QGIS cannot use
any."""

from __future__ import annotations

import collections.abc as c
import importlib
import platform
import sys

import pytest

from topology_rules.core import wheels

from .wheel_helpers import (
    VERSION,
    WHEELS,
    answer,
    listed,
    mac,
    pretend_linux,
    release_file,
    wheel_name,
    windows,
)


@pytest.fixture(autouse=True)
def linux(monkeypatch: pytest.MonkeyPatch) -> None:
    pretend_linux(monkeypatch)


# The wheel that fits


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
    release = wheels.pick(listed(), VERSION)
    assert release.filename == wheel_name(WHEELS[expected])
    assert release.url == f'https://files.invalid/{expected}'


@pytest.mark.parametrize(
    ('machine', 'expected'), [('arm64', 'mac_arm'), ('x86_64', 'mac_intel')]
)
def test_a_python_that_says_macos_10_16_gets_a_wheel(
    monkeypatch: pytest.MonkeyPatch, machine: str, expected: str
):
    # What a Python built with an older SDK says on macOS 11 and newer
    mac(monkeypatch, machine, '10.16')
    release = wheels.pick(listed(), VERSION)
    assert release.filename == wheel_name(WHEELS[expected])


def test_macos_10_16_is_macos_11():
    assert wheels.mac_tags('arm64', (10, 16)) == [
        'macosx_11_0_arm64',
        'macosx_11_0_universal2',
    ]
    assert wheels.mac_tags('x86_64', (10, 16)) == wheels.mac_tags(
        'x86_64', (11, 0)
    )


def test_an_older_macos_than_the_wheel_has_none(
    monkeypatch: pytest.MonkeyPatch,
):
    mac(monkeypatch, 'x86_64', '10.11')
    with pytest.raises(wheels.Failed, match='no wheel'):
        wheels.pick(listed(), VERSION)


def test_the_tags_of_a_glibc():
    assert wheels.linux_tags((2, 17)) == [
        'manylinux_2_17_x86_64',
        'manylinux2014_x86_64',
        *[f'manylinux_2_{minor}_x86_64' for minor in range(16, 12, -1)],
        'manylinux_2_12_x86_64',
        'manylinux2010_x86_64',
        *[f'manylinux_2_{minor}_x86_64' for minor in range(11, 5, -1)],
        'manylinux_2_5_x86_64',
        'manylinux1_x86_64',
    ]


@pytest.mark.parametrize('glibc', [(), (2,), (1, 9), (3, 1)])
def test_a_glibc_without_a_minor_version_or_of_another_major_has_no_tags(
    glibc: tuple[int, ...],
):
    assert wheels.linux_tags(glibc) == []


def test_a_wheel_for_another_python_is_not_chosen():
    tag = 'cp313-cp313-manylinux_2_17_x86_64'
    data = answer([release_file(tag, b'x', 'https://files.invalid/313')])
    with pytest.raises(wheels.Failed, match='no wheel'):
        wheels.pick(data, VERSION)


def test_another_version_from_pypi_is_refused():
    data = listed()
    data['info']['version'] = '0.0.1'
    with pytest.raises(wheels.Failed, match=r'0\.0\.1 for '):
        wheels.pick(data, VERSION)


def test_a_file_without_a_sha256_is_refused():
    data = listed()
    for file in data['urls']:
        file['digests'] = {}
    with pytest.raises(wheels.Failed, match='SHA-256'):
        wheels.pick(data, VERSION)


def test_a_release_holds_what_the_consent_shows():
    data = listed()
    release = wheels.pick(data, VERSION)
    linux = next(f for f in data['urls'] if f['filename'] == release.filename)
    assert release.version == VERSION
    assert release.size == linux['size']
    assert release.sha256 == linux['digests']['sha256']


# Why this QGIS cannot use the package


def test_this_qgis_can_use_it_without_shapely_or_pyproj():
    # The linux fixture hides both
    for name in ('shapely', 'pyproj'):
        with pytest.raises(ImportError):
            importlib.import_module(name)
    assert wheels.unsupported() is None


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
            lambda m: m.setattr(platform, 'libc_ver', lambda: ('glibc', '2')),
            'Needs glibc 2.17 or newer, Linux has glibc 2',
        ),
        (
            lambda m: m.setattr(platform, 'system', lambda: 'FreeBSD'),
            'No wheel for FreeBSD x86_64',
        ),
    ],
)
def test_why_this_qgis_cannot_use_it_is_said(
    monkeypatch: pytest.MonkeyPatch,
    patch: c.Callable[[pytest.MonkeyPatch], None],
    reason: str,
):
    patch(monkeypatch)
    found = wheels.unsupported()
    assert found is not None
    assert found.startswith(reason)
