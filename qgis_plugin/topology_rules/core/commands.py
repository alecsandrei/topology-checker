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
# How much of a report that cannot be read the log shows
SHOWN_CHARS = 200


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
    stream: bool = True,
) -> list[str]:
    """The command of a run, whose report is on the output. With stream the
    tool says the report of each rule as soon as the rule is done."""
    command = [executable, '--json', 'validate', config]
    if stream:
        command.append('--stream')
    if tolerance is not None:
        command += ['--tolerance', repr(tolerance)]
    return command


def definition_command(
    executable: str, config: str, tolerance: float | None, output: str
) -> list[str]:
    """The command of a run that writes the errors to the output file, and
    only counts them in the report."""
    command = [executable, '--json', 'validate', config, '--summary']
    if tolerance is not None:
        command += ['--tolerance', repr(tolerance)]
    return [*command, '-o', output, '--overwrite']


# The executables that do not know --stream, by path and time of change, so
# that one that is replaced is asked again
WITHOUT_STREAM: set[tuple[str, int]] = set()
# What the parser of the arguments of an executable from before --stream
# says of the flag, on a line of its own, and the colors it may have
REFUSED = re.compile(r"^error: unexpected argument '--stream' found\r?$", re.M)
COLORS = re.compile(r'\x1b\[[0-9;]*m')


def modified(executable: str) -> int:
    try:
        return os.stat(executable).st_mtime_ns
    except OSError:
        return 0


def streams(executable: str) -> bool:
    """Whether the executable is not known to say its report at the end
    only."""
    return (executable, modified(executable)) not in WITHOUT_STREAM


def run_validate(
    executable: str,
    config: str,
    tolerance: float | None,
    feedback: QgsProcessingFeedback,
    out: Lines,
    older: c.Callable[[], None] | None = None,
    done: c.Callable[[], bool] | None = None,
) -> Completed:
    """Runs the rules of a definition, giving `out` each line of the report
    as soon as the executable says it. Once `done` says that the report is
    complete, the executable has a moment to end before it is ended.

    An executable that does not know --stream is asked again without it, and
    says the whole report at the end; it is not asked with the flag again,
    and `older` hears of it. Only the words of its parser tell that it does
    not know it: other errors may name the flag, as a layer may.
    """
    version = (executable, modified(executable))
    stream = version not in WITHOUT_STREAM
    while True:
        taken = False

        def read(line: str) -> bool:
            nonlocal taken
            took = out(line)
            taken = taken or took
            return took

        completed = execute(
            validate_command(executable, config, tolerance, stream),
            feedback,
            out=read,
            done=done,
        )
        refused = (
            stream
            and not taken
            and completed.exit_code == ExitCode.ERROR.value
            and REFUSED.search(COLORS.sub('', completed.stderr)) is not None
        )
        if not refused:
            return completed
        WITHOUT_STREAM.add(version)
        stream = False
        if older is not None:
            older()


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


def rule_counts(
    completed: Completed, feedback: QgsProcessingFeedback
) -> list[int]:
    """The number of errors of each rule of a run, in the order of the run.

    Raises if the run failed, or if its report cannot be read, the start of
    which the log shows.
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
        start = completed.stdout.strip()
        if len(start) > SHOWN_CHARS:
            start = start[:SHOWN_CHARS] + '…'
        feedback.pushInfo(
            tr('The report starts with: {text}').format(text=start)
        )
        raise RunError(tr("Could not read the executable's report.")) from error
