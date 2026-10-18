from __future__ import annotations

import dataclasses
import json
import os
import typing as t
from pathlib import Path

from ..enums import ExitCode
from ..exceptions import RunError
from .binary import Rule
from .layers import Dataset
from .runner import Completed, error_message

if t.TYPE_CHECKING:
    from .._typing import ReportJson

ERROR_LAYERS = ('errors_point', 'errors_line', 'errors_polygon')


@dataclasses.dataclass(frozen=True)
class Check:
    """A rule and the data it runs on, as one entry of a `validate` run."""

    rule: Rule
    layer: Dataset
    other: Dataset | None = None


def quoted(text: str) -> str:
    """A TOML basic string, which JSON escaping also produces."""
    return json.dumps(text, ensure_ascii=False)


def shown_path(source: str, config: Path, relative: bool) -> str:
    """A path as the file names it: relative to the folder of the file if
    asked, which the tool resolves against the file, else as it is."""
    if not relative:
        return source
    try:
        return os.path.relpath(source, config.parent).replace(os.sep, '/')
    except ValueError:
        # On another drive, so there is no way from one to the other
        return source


def write_config(
    path: Path,
    checks: t.Sequence[Check],
    tolerance: float | None = None,
    relative: bool = False,
    names: t.Mapping[Dataset, str] | None = None,
) -> dict[Dataset, str]:
    """Writes the TOML definition that `validate` reads, and returns the alias
    of each dataset.

    Each dataset is a layer alias, named as the names say, else after its
    layer or file, and made unique, which the report shows as the origin layer.
    """
    aliases: dict[Dataset, str] = {}
    taken: set[str] = set()
    for check in checks:
        for source in (check.layer, check.other):
            if source is None or source in aliases:
                continue
            base = (names or {}).get(source) or (
                source.layer or Path(source.path).stem
            )
            alias, index = base, 1
            while alias in taken:
                index += 1
                alias = f'{base}_{index}'
            taken.add(alias)
            aliases[source] = alias
    lines = [] if tolerance is None else [f'tolerance = {tolerance!r}', '']
    lines.append('[layers]')
    for source, alias in aliases.items():
        entry = f'path = {quoted(shown_path(source.path, path, relative))}'
        if source.layer is not None:
            entry += f', layer = {quoted(source.layer)}'
        lines.append(f'{quoted(alias)} = {{ {entry} }}')
    for check in checks:
        lines += [
            '',
            '[[rules]]',
            f'rule = {quoted(check.rule.name)}',
            f'layer = {quoted(aliases[check.layer])}',
        ]
        if check.other is not None:
            lines.append(f'other = {quoted(aliases[check.other])}')
    path.write_text('\n'.join(lines) + '\n', encoding='utf-8')
    return aliases


def validate_command(
    executable: str, config: str, tolerance: float | None, output: str
) -> list[str]:
    command = [executable, '--json', 'validate', config]
    if tolerance is not None:
        command += ['--tolerance', repr(tolerance)]
    return [*command, '-o', output, '--overwrite']


def utility_command(
    executable: str, name: str, source: Dataset, output: str
) -> list[str]:
    return [
        executable,
        '--overwrite',
        'utilities',
        name,
        *source.arguments('--layer'),
        '-o',
        output,
    ]


def rule_counts(completed: Completed) -> list[int]:
    """The number of errors of each rule of a run, in the order of the run.

    Raises if the run failed.
    """
    if completed.exit_code not in (
        ExitCode.VALID.value,
        ExitCode.VIOLATIONS.value,
    ):
        raise RunError(error_message(completed))
    try:
        report = t.cast('ReportJson', json.loads(completed.stdout))
        return [sum(rule['counts'].values()) for rule in report['rules']]
    except (ValueError, KeyError, TypeError) as error:
        raise RunError(
            f'Could not read the report: {completed.stdout[:200]!r}'
        ) from error


def count_errors(completed: Completed) -> int:
    """The number of errors in a run, raising if the run failed."""
    return sum(rule_counts(completed))
