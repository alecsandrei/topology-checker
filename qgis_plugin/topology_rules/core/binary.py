from __future__ import annotations

import dataclasses
import json
import os
import shutil
import sys
import threading
import typing as t
from concurrent.futures import Future
from pathlib import Path

from qgis.core import QgsSettings

from ..enums import EXECUTABLE_NAME, ExitCode, Group, Setting
from ..exceptions import ExecutableError, RunError
from ..utils import title
from .runner import error_message, run

if t.TYPE_CHECKING:
    from .._typing import RuleJson


@dataclasses.dataclass(frozen=True)
class Rule:
    name: str
    description: str
    # Geometry kinds of the first and the optional second layer: 'point',
    # 'line', 'polygon' or 'any'
    layer: str
    other: str | None

    @property
    def group(self) -> Group:
        return Group(self.name.split('/')[0])

    @property
    def slug(self) -> str:
        return self.name.replace('/', '_').replace('-', '_')

    @property
    def display_name(self) -> str:
        return display_name(self.name)

    @property
    def short_name(self) -> str:
        return short_name(self.name)

    def fits(self, kind: str) -> bool:
        """Whether the first layer of the rule can be of the geometry kind."""
        return self.layer in ('any', kind)


# Seconds the executable has to list its rules
# The first start of a program can take this long, with a scan by the system
PROBE_TIMEOUT = 30
MAX_VERSIONS = 4


def display_name(name: str) -> str:
    """'line/must-not-have-dangles' as 'Lines must not have dangles', which
    says what the rule is about wherever the rule is named."""
    group, _, rest = name.partition('/')
    try:
        noun = Group(group).noun
    except ValueError:
        return title(rest or name)
    words = rest.split('-')
    if 'must' in words[1:]:
        # 'endpoints-must-be-covered-by-points' reads as 'must have
        # endpoints covered by points', like the other names
        at = words.index('must')
        verb = words[at + 1 :]
        if verb[:1] == ['be']:
            verb = verb[1:]
        words = ['must', 'have', *words[:at], *verb]
    return f'{noun} {" ".join(words)}'


def short_name(name: str) -> str:
    """'line/must-not-have-dangles' as 'Must not have dangles', for where
    the geometry is already on the screen."""
    full = display_name(name)
    group = name.partition('/')[0]
    try:
        noun = Group(group).noun
    except ValueError:
        return full
    rest = full[len(noun) + 1 :]
    return rest[:1].upper() + rest[1:]


def named_with(short: str, other: str) -> str:
    """The name of a rule that takes a second layer, with the layer."""
    if short.endswith(' with'):
        return f'{short} {other}'
    return f'{short} with {other}'


def configured_path() -> str:
    """The path of the setting, as pasted: maybe quoted or with a `~`."""
    value = QgsSettings().value(Setting.EXECUTABLE.value, '')
    text = str(value or '').strip()
    if len(text) > 1 and text[0] == text[-1] and text[0] in '"\'':
        text = text[1:-1].strip()
    return os.path.expanduser(text)


def find_executable() -> Path:
    """The configured executable, or the one on the PATH."""
    configured = configured_path()
    if configured:
        path = Path(configured)
        if path.is_dir():
            raise ExecutableError(
                f'This is a folder, not the topology-checker executable: {path}'
            )
        # A path pasted without the extension that Windows hides
        program = Path(f'{configured}.exe')
        if (
            not path.is_file()
            and sys.platform == 'win32'
            and path.suffix.lower() != '.exe'
            and program.is_file()
        ):
            return program
        if not path.is_file():
            raise ExecutableError(
                f'The topology-checker executable does not exist: {path}'
            )
        return path
    found = shutil.which(EXECUTABLE_NAME)
    if found is None:
        raise ExecutableError(
            'topology-checker was not found. Install it, then set its path '
            'under Settings > Options > Processing > Providers > '
            'Topology Rules.'
        )
    return Path(found)


def known(entry: RuleJson) -> bool:
    """Whether the plugin can list the rule, which has a name of the shape
    `group/rule` and a group of the plugin."""
    parts = str(entry['name']).split('/')
    return (
        len(parts) == 2
        and all(parts)
        and parts[0] in {group.value for group in Group}
    )


# What to do about an executable that cannot be used
CHOOSE = (
    "Choose a newer build with 'Choose topology-checker…' or under "
    'Settings > Options > Processing > Providers > Topology Rules.'
)


def parse_rules(text: str, path: str = '') -> tuple[Rule, ...]:
    """Reads the output of `topology-checker --json rules`."""
    try:
        entries = t.cast('list[RuleJson]', json.loads(text))
        if not isinstance(entries, list):
            raise TypeError('not a list')
        rules = tuple(
            Rule(
                entry['name'],
                entry['description'],
                entry['layer'],
                entry['other'],
            )
            for entry in entries
            if known(entry)
        )
    except (ValueError, KeyError, TypeError) as error:
        raise ExecutableError(
            f'{who(path)} is too old for the plugin, which needs '
            'topology-checker 0.2.0 or newer: it does not list the layers '
            f'each rule needs. {CHOOSE}'
        ) from error
    if not rules:
        raise ExecutableError(f'{who(path)} lists no rules. {CHOOSE}')
    return rules


def who(path: str) -> str:
    """The executable, by its path where it is known."""
    return (
        f'The topology-checker at {path}' if path else 'This topology-checker'
    )


class Probe:
    """Asks an executable for its rules, once for each version of it.

    The first answer can take seconds, as a program that was just installed
    is often checked by the system before it runs, so it can be asked from
    another thread, and a version that was asked before is answered at once.
    """

    def __init__(self) -> None:
        self.done: dict[tuple[str, int], tuple[Rule, ...]] = {}
        # The file extensions that each version reads, asked with the rules
        self.readable: dict[tuple[str, int], frozenset[str]] = {}
        # The versions that are being asked, for a second request to share
        self.pending: dict[tuple[str, int], Future[tuple[Rule, ...]]] = {}
        # The answers are read on the main thread and written by the thread
        # that asked
        self.lock = threading.Lock()

    def __call__(self, path: str, modified: int) -> tuple[Rule, ...]:
        key = (path, modified)
        with self.lock:
            found = self.done.get(key)
            if found is not None:
                return found
            future = self.pending.get(key)
            asking = future is None
            if future is None:
                future = self.pending[key] = Future()
        if not asking:
            return future.result()
        try:
            # Not under the lock, as the answer can take seconds
            found = self.ask(path)
        except BaseException as error:
            with self.lock:
                del self.pending[key]
            future.set_exception(error)
            raise
        with self.lock:
            self.done[key] = found
            del self.pending[key]
            # A few versions are enough, as the path changes by hand
            while len(self.done) > MAX_VERSIONS:
                gone = next(iter(self.done))
                del self.done[gone]
                self.readable.pop(gone, None)
        future.set_result(found)
        # The formats are asked on the side: they are not needed to list the
        # rules, and a tool that is slow to answer must not hold them up
        threading.Thread(
            target=self.learn_formats, args=(key,), daemon=True
        ).start()
        return found

    def learn_formats(self, key: tuple[str, int]) -> None:
        formats = self.ask_formats(key[0])
        with self.lock:
            if formats is not None and key in self.done:
                self.readable[key] = formats

    def peek(self, path: str, modified: int) -> tuple[Rule, ...] | None:
        with self.lock:
            return self.done.get((path, modified))

    def formats(self, path: str, modified: int) -> frozenset[str] | None:
        with self.lock:
            return self.readable.get((path, modified))

    def cache_clear(self) -> None:
        with self.lock:
            self.done.clear()
            self.readable.clear()

    @staticmethod
    def ask_formats(path: str) -> frozenset[str] | None:
        """The extensions of the files that the executable reads, or none if
        it does not say."""
        try:
            completed = run(
                [path, '--json', 'drivers', '--read'], timeout=PROBE_TIMEOUT
            )
            if completed.exit_code != ExitCode.VALID.value:
                return None
            found = json.loads(completed.stdout)
            return frozenset(
                str(extension).lower().lstrip('.')
                for driver in found
                if driver.get('read')
                for extension in driver.get('extensions', [])
            )
        except (RunError, ValueError, AttributeError, TypeError):
            return None

    @staticmethod
    def ask(path: str) -> tuple[Rule, ...]:
        try:
            completed = run([path, '--json', 'rules'], timeout=PROBE_TIMEOUT)
        except RunError as error:
            raise ExecutableError(f'{error} {CHOOSE}') from error
        if completed.exit_code != ExitCode.VALID.value:
            raise ExecutableError(
                f'{who(path)} does not work: '
                f'{error_message(completed)} {CHOOSE}'
            )
        return parse_rules(completed.stdout, path)


_rules = Probe()
# What the dock asks, to see if an answer is there already
probe_cache = _rules


def probe(path: str, modified: int) -> tuple[Rule, ...]:
    """The rules of the executable, to ask from any thread."""
    return _rules(path, modified)


def locate() -> tuple[str, int]:
    """The executable and its version, which reads the settings and so
    belongs to the main thread."""
    path = find_executable()
    try:
        modified = path.stat().st_mtime_ns
    except OSError as error:
        raise ExecutableError(f'Could not read {path}: {error}') from error
    return str(path), modified


# What the executable reads when it was not asked: the formats it has of its
# own
NATIVE_EXTENSIONS = frozenset(
    {
        'shp',
        'fgb',
        'gpkg',
        'gpx',
        'geojson',
        'json',
        'parquet',
        'geoparquet',
        'kml',
        'kmz',
    }
)


def readable_extensions() -> frozenset[str]:
    """The extensions of the files the executable reads, for the main
    thread."""
    try:
        path, modified = locate()
    except ExecutableError:
        return NATIVE_EXTENSIONS
    return _rules.formats(path, modified) or NATIVE_EXTENSIONS


def rules() -> tuple[Rule, ...]:
    """The rules of the executable, read again when it changes."""
    return _rules(*locate())
