from __future__ import annotations

import collections.abc as c
import dataclasses
import json
import os
import re
import typing as t
from pathlib import Path

from ..enums import ExitCode
from ..exceptions import RunError
from ..utils import tr
from .binary import Rule
from .layers import Dataset
from .runner import Completed, Lines, error_message, execute

if t.TYPE_CHECKING:
    from qgis.core import QgsProcessingFeedback

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


def unique(base: str, taken: set[str]) -> str:
    """The name, or the first of `name_2`, `name_3` and so on that is not
    taken, which it takes."""
    name, index = base, 1
    while name in taken:
        index += 1
        name = f'{base}_{index}'
    taken.add(name)
    return name


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
            aliases[source] = unique(
                (names or {}).get(source) or source.name, taken
            )
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
    executable: str,
    config: str,
    tolerance: float | None,
    output: str,
    summary: bool = False,
    progress: bool = False,
) -> list[str]:
    """The command of a run. With a summary the report leaves out the
    violations, which are in the output file, and only counts them. With
    progress the tool says when each rule is done."""
    command = [executable, '--json', 'validate', config]
    if summary:
        command.append('--summary')
    if progress:
        command.append('--progress')
    if tolerance is not None:
        command += ['--tolerance', repr(tolerance)]
    return [*command, '-o', output, '--overwrite']


# The executables that do not know --summary, which older versions lack
WITHOUT_SUMMARY: set[str] = set()
# The executables that do not know --progress, by path and time of change, so
# that one that is replaced is asked again
WITHOUT_PROGRESS: set[tuple[str, int]] = set()
# What the tool says with --progress when a rule is done
PROGRESS = re.compile(r'checked rule (\d+)/(\d+) \((.*)\): (\d+) errors?')

# A rule that is done: its index, the number of rules, its errors
Done = c.Callable[[int, int, int], None]


def progress_of(line: str) -> tuple[int, int, int] | None:
    """The index of the rule that a line of --progress says is done, the
    number of rules and the errors of the rule."""
    match = PROGRESS.fullmatch(line.strip())
    if match is None:
        return None
    number, total, count = (int(match[i]) for i in (1, 2, 4))
    if not 1 <= number <= total:
        return None
    return number - 1, total, count


def modified(executable: str) -> int:
    try:
        return os.stat(executable).st_mtime_ns
    except OSError:
        return 0


def run_validate(
    executable: str,
    config: str,
    tolerance: float | None,
    output: str,
    feedback: QgsProcessingFeedback,
    done: Done | None = None,
) -> Completed:
    """Runs the rules of a definition and returns the report, asking for no
    list of the violations where the executable can leave it out.

    Done, if given, is told of each rule as soon as it is done, where the
    executable can say so. An executable that refuses a flag is asked
    without it, and not with it again.
    """
    lines: Lines | None = None
    if done is not None:
        told = done

        def lines(line: str) -> bool:
            found = progress_of(line)
            if found is not None:
                told(*found)
            return found is not None

    version = (executable, modified(executable))
    summary = executable not in WITHOUT_SUMMARY
    progress = lines is not None and version not in WITHOUT_PROGRESS
    while True:
        completed = execute(
            validate_command(
                executable, config, tolerance, output, summary, progress
            ),
            feedback,
            lines if progress else None,
        )
        said = completed.stderr + completed.stdout
        if completed.exit_code != ExitCode.ERROR.value:
            return completed
        if summary and '--summary' in said:
            WITHOUT_SUMMARY.add(executable)
            summary = False
        elif progress and '--progress' in said:
            WITHOUT_PROGRESS.add(version)
            progress = False
        else:
            return completed


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
            tr('Could not read the report: {report}').format(
                report=repr(completed.stdout[:200])
            )
        ) from error


def count_errors(completed: Completed) -> int:
    """The number of errors in a run, raising if the run failed."""
    return sum(rule_counts(completed))
