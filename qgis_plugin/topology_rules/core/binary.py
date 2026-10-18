from __future__ import annotations

import dataclasses
import json
import os
import shutil
import sys
import threading
import typing as t
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


def parse_rules(text: str) -> tuple[Rule, ...]:
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
            'This topology-checker is too old for the plugin. Update it.'
        ) from error
    if not rules:
        raise ExecutableError('This topology-checker lists no rules.')
    return rules


class Probe:
    """Asks an executable for its rules, once for each version of it.

    The first answer can take seconds, as a program that was just installed
    is often checked by the system before it runs, so it can be asked from
    another thread, and a version that was asked before is answered at once.
    """

    def __init__(self) -> None:
        self.done: dict[tuple[str, int], tuple[Rule, ...]] = {}
        # The answers are read on the main thread and written by the thread
        # that asked
        self.lock = threading.Lock()

    def __call__(self, path: str, modified: int) -> tuple[Rule, ...]:
        found = self.peek(path, modified)
        if found is None:
            # Not under the lock, as the answer can take seconds
            found = self.ask(path)
            with self.lock:
                self.done[(path, modified)] = found
                # A few versions are enough, as the path changes by hand
                while len(self.done) > MAX_VERSIONS:
                    del self.done[next(iter(self.done))]
        return found

    def peek(self, path: str, modified: int) -> tuple[Rule, ...] | None:
        with self.lock:
            return self.done.get((path, modified))

    def cache_clear(self) -> None:
        with self.lock:
            self.done.clear()

    @staticmethod
    def ask(path: str) -> tuple[Rule, ...]:
        try:
            completed = run([path, '--json', 'rules'], timeout=PROBE_TIMEOUT)
        except RunError as error:
            raise ExecutableError(str(error)) from error
        if completed.exit_code != ExitCode.VALID.value:
            raise ExecutableError(error_message(completed))
        return parse_rules(completed.stdout)


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


def rules() -> tuple[Rule, ...]:
    """The rules of the executable, read again when it changes."""
    return _rules(*locate())
