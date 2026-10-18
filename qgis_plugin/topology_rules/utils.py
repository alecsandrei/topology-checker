from __future__ import annotations

import typing as t

from qgis import core as _core
from qgis.core import Qgis, QgsProcessing, QgsProcessingParameterNumber


def named(owner: object, name: str) -> t.Any:
    """A name that some versions of QGIS have and their stubs do not."""
    return getattr(owner, name)


def source_type(kind: str) -> t.Any:
    """A vector source type, such as 'VectorPoint', on every QGIS version."""
    scoped = getattr(Qgis, 'ProcessingSourceType', None)
    if scoped is not None:
        return getattr(scoped, kind)
    return getattr(QgsProcessing, f'Type{kind}')


def double_type() -> t.Any:
    scoped = getattr(Qgis, 'ProcessingNumberParameterType', None)
    if scoped is not None:
        return scoped.Double
    return named(QgsProcessingParameterNumber, 'Double')


def title(slug: str) -> str:
    """'must-not-have-dangles' as 'Must not have dangles'."""
    text = slug.replace('-', ' ')
    return text[:1].upper() + text[1:]


V = t.TypeVar('V')


# Not a type parameter list, which needs a newer Python than QGIS may have
def not_none(value: V | None) -> V:  # noqa: UP047
    """The value, which the QGIS stubs type as optional but is set."""
    if value is None:
        raise ValueError('Did not expect None')
    return value


# The stubs of some QGIS versions miss the exception that Processing defines
QgsProcessingException: type[Exception] = named(_core, 'QgsProcessingException')
QgsCsException: type[Exception] = named(_core, 'QgsCsException')
