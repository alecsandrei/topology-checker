"""Why a topology.toml cannot be read, in words that are translated."""

from __future__ import annotations

from pathlib import Path

import pytest

from topology_rules.core.binary import Rule
from topology_rules.core.definition import DefinitionError, read_definition

DANGLES = 'line/must-not-have-dangles'
KNOWN = {DANGLES: Rule(DANGLES, '', 'line', None)}
LAYERS = "[layers]\na = { path = 'a.gpkg' }\n"
RULE = f"[[rules]]\nrule = '{DANGLES}'\nlayer = 'a'\n"

pytestmark = pytest.mark.usefixtures('marked')


def read(tmp_path: Path, text: str) -> str:
    """Why the definition of the text cannot be read."""
    path = tmp_path / 'topology.toml'
    path.write_text(text, encoding='utf-8')
    with pytest.raises(DefinitionError) as raised:
        read_definition(path, KNOWN)
    return str(raised.value)


@pytest.mark.parametrize(
    ('text', 'message'),
    [
        (f'colour = 1\n{LAYERS}{RULE}', '<Unknown keys: colour>'),
        ('tolerance = 1\n', '<topology.toml needs [layers] and [[rules]].>'),
        (
            f'tolerance = -1\n{LAYERS}{RULE}',
            '<The tolerance must be a number, not negative.>',
        ),
        (f'rules = [1]\n{LAYERS}', '<Each rule must be a table.>'),
        (f'{LAYERS}{RULE}colour = 1\n', '<Unknown keys of a rule: colour>'),
        (
            f"{LAYERS}[[rules]]\nrule = 'line/nope'\nlayer = 'a'\n",
            '<Unknown rule: line/nope>',
        ),
        (
            f"{LAYERS}{RULE}other = 'a'\n",
            f'<Wrong number of layers for {DANGLES}.>',
        ),
        (
            f"{LAYERS}[[rules]]\nrule = '{DANGLES}'\nlayer = 'b'\n",
            '<Unknown layer: b>',
        ),
        (
            "[layers]\na = { path = 'a.gpkg', colour = 1 }\n" + RULE,
            '<Unknown keys of layer a: colour>',
        ),
        (LAYERS + RULE, '<The data of a is missing: {where}>'),
    ],
    ids=[
        'top keys',
        'no rules',
        'tolerance',
        'rule not a table',
        'rule keys',
        'rule',
        'layer count',
        'layer',
        'layer keys',
        'data',
    ],
)
def test_why_a_definition_cannot_be_read_is_translated(
    tmp_path: Path, text: str, message: str
):
    where = (tmp_path / 'a.gpkg').resolve()
    assert read(tmp_path, text) == message.format(where=where)


def test_why_its_data_cannot_be_read_is_translated(tmp_path: Path):
    (tmp_path / 'a.gpkg').write_text('not a GeoPackage')
    assert read(tmp_path, LAYERS + RULE) == '<Cannot read the data of a.>'


def test_why_it_is_not_toml_is_translated(tmp_path: Path):
    found = read(tmp_path, 'x =')
    assert found.startswith('<topology.toml is not valid TOML: ')
    assert found.endswith('>')


def test_why_the_file_cannot_be_read_is_translated(tmp_path: Path):
    path = tmp_path / 'nothing.toml'
    with pytest.raises(DefinitionError) as raised:
        read_definition(path, KNOWN)
    assert str(raised.value).startswith(f'<Cannot read {path}: ')
    assert str(raised.value).endswith('>')
