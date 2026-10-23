from __future__ import annotations

from enum import Enum
from pathlib import Path

_ASSETS_DIR = Path(__file__).parent / 'assets'

PROVIDER_ID = 'topologyrules'
EXECUTABLE_NAME = 'topology-checker'
# The name of the setting of the executable, which Processing keeps under its
# own prefix
SETTING_NAME = 'TOPOLOGY_RULES_EXECUTABLE'


class Asset(Enum):
    ICON = _ASSETS_DIR / 'icon.svg'


class Setting(Enum):
    # Processing stores its provider settings under this prefix
    EXECUTABLE = f'Processing/Configuration/{SETTING_NAME}'
    GEOMETRY = 'TopologyRules/geometry'
    # What runs the checks: 'automatic', 'package' or 'executable'
    ENGINE = 'TopologyRules/engine'


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
