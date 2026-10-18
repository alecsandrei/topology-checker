from __future__ import annotations

import dataclasses
import tomllib
import typing as t
from pathlib import Path

from qgis.core import QgsProject, QgsVectorLayer

from ..exceptions import RunError
from ..utils import not_none
from .binary import Rule
from .layers import plain_dataset, same_file

TOP = {'tolerance', 'layers', 'rules'}
LAYER_KEYS = {'path', 'layer'}
RULE_KEYS = {'rule', 'layer', 'other'}


class DefinitionError(Exception):
    """A definition cannot be written or read, and says why."""


@dataclasses.dataclass
class Imported:
    """The rules of a definition on layers that are ready to use: those of
    the project, and those that the file adds."""

    rules: list[tuple[Rule, QgsVectorLayer, QgsVectorLayer | None]]
    new_layers: list[QgsVectorLayer]
    tolerance: float | None


def unknown(keys: t.Iterable[str], allowed: set[str], where: str) -> None:
    extra = sorted(set(keys) - allowed)
    if extra:
        raise DefinitionError(f'Unknown {where}: {", ".join(extra)}')


def project_match(path: Path, name: str | None) -> QgsVectorLayer | None:
    """The layer of the project that reads the layer of the file."""
    layers = not_none(QgsProject.instance()).mapLayers().values()
    for layer in layers:
        if not isinstance(layer, QgsVectorLayer):
            continue
        try:
            found = plain_dataset(layer)
        except RunError:
            continue
        if found and same_file(found.path, str(path)) and found.layer == name:
            return layer
    return None


def read_definition(path: Path, known: dict[str, Rule]) -> Imported:
    """Reads and checks a definition, so that nothing has to be undone: the
    layers are not added to the project here."""
    try:
        config = tomllib.loads(path.read_text(encoding='utf-8'))
    except OSError as error:
        raise DefinitionError(
            f'Cannot read {path}: {error.strerror}'
        ) from error
    except (tomllib.TOMLDecodeError, UnicodeDecodeError) as error:
        raise DefinitionError(
            f'{path.name} is not valid TOML: {error}'
        ) from error
    unknown(config, TOP, 'keys')
    sources = config.get('layers')
    rules = config.get('rules')
    if (
        not isinstance(sources, dict)
        or not isinstance(rules, list)
        or not rules
    ):
        raise DefinitionError(f'{path.name} needs [layers] and [[rules]].')
    tolerance = config.get('tolerance')
    if tolerance is not None and not (
        isinstance(tolerance, (int, float)) and tolerance >= 0
    ):
        raise DefinitionError('The tolerance must be a number, not negative.')
    layers: dict[str, QgsVectorLayer] = {}
    new_layers: list[QgsVectorLayer] = []

    def layer_of(alias: object) -> QgsVectorLayer:
        found = sources.get(str(alias))
        if not isinstance(found, dict):
            raise DefinitionError(f'Unknown layer: {alias}')
        if str(alias) in layers:
            return layers[str(alias)]
        unknown(found, LAYER_KEYS, f'keys of layer {alias}')
        # A file written on Windows may have backslashes
        relative = str(found.get('path', '')).replace('\\', '/')
        where = (path.parent / relative).resolve()
        name = found.get('layer')
        if not where.is_file():
            raise DefinitionError(f'The data of {alias} is missing: {where}')
        layer = project_match(where, name)
        if layer is None:
            uri = f'{where}|layername={name}' if name else str(where)
            layer = QgsVectorLayer(uri, str(alias), 'ogr')
            if not layer.isValid():
                raise DefinitionError(f'Cannot read the data of {alias}.')
            new_layers.append(layer)
        layers[str(alias)] = layer
        return layer

    chosen: list[tuple[Rule, QgsVectorLayer, QgsVectorLayer | None]] = []
    for entry in rules:
        if not isinstance(entry, dict):
            raise DefinitionError('Each rule must be a table.')
        unknown(entry, RULE_KEYS, 'keys of a rule')
        rule = known.get(str(entry.get('rule')))
        if rule is None:
            raise DefinitionError(f'Unknown rule: {entry.get("rule")}')
        other = entry.get('other')
        if (other is None) != (rule.other is None):
            raise DefinitionError(f'Wrong number of layers for {rule.name}.')
        chosen.append(
            (
                rule,
                layer_of(entry.get('layer')),
                None if other is None else layer_of(other),
            )
        )
    return Imported(
        chosen, new_layers, None if tolerance is None else float(tolerance)
    )
