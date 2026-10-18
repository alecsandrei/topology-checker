from __future__ import annotations

from enum import Enum
from pathlib import Path

_ASSETS_DIR = Path(__file__).parent / 'assets'

PROVIDER_ID = 'topologyrules'
EXECUTABLE_NAME = 'topology-checker'


class Asset(Enum):
    ICON = _ASSETS_DIR / 'icon.svg'


class Setting(Enum):
    # Processing stores its provider settings under this prefix
    EXECUTABLE = 'Processing/Configuration/TOPOLOGY_RULES_EXECUTABLE'
    GEOMETRY = 'TopologyRules/geometry'


class Group(Enum):
    POINT = 'point'
    LINE = 'line'
    POLYGON = 'polygon'
    GEOMETRY = 'geometry'

    @property
    def label(self) -> str:
        return {
            Group.POINT: 'Points',
            Group.LINE: 'Lines',
            Group.POLYGON: 'Polygons',
            Group.GEOMETRY: 'All geometries',
        }[self]

    @property
    def noun(self) -> str:
        """What the rules of the group are about, to start their names."""
        return {
            Group.POINT: 'Points',
            Group.LINE: 'Lines',
            Group.POLYGON: 'Polygons',
            Group.GEOMETRY: 'Geometries',
        }[self]


class ExitCode(Enum):
    VALID = 0
    VIOLATIONS = 1
    ERROR = 2
