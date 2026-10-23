"""Topology Rules: the rules of topology-checker in QGIS.

The plugin runs the checks with the topology-checker Python package or the
executable, and shows what they find.

* plugin, provider: start and stop the plugin; the Processing provider
* enums, exceptions, utils: what the other modules share
* core/: the work, with no widgets and no QGIS window
  * engine: which of the two runs the checks
  * package: a run of the Python package
  * binary, runner: find the executable, list its rules, run it
  * commands, definition: its command lines, and the topology.toml it reads
  * layers, inputs: what a layer is to the tool, and the files made for it
  * checks, invalid: runs rules in one pass; the features it would reject
  * style: how errors are drawn and grouped
  * install, wheels, folders: the Python package from PyPI; the wheel that
    fits this QGIS; its folders in the profile
* algorithms/: the Processing algorithms (rule, check, definition, utilities)
  with base and sinks for what they share
* panel/: the dock
  * dock: the panel; rules_page (form, rule_list, marks, options), results
    (tree, ignored), setup and hint are its parts
  * entries, rows: the rules of the list and the errors, as data
  * controller, startup: the run, and asking the tool in the background
  * menu, installer, offer: the gear menu of the engine; the install of the
    Python package; the bar that offers it
  * exchange, state: topology.toml and the project
  * groups, focus, messages, icons, tolerance: map layers, zoom, messages,
    button icons, tolerance
"""

from __future__ import annotations

import typing as t

if t.TYPE_CHECKING:
    from qgis.gui import QgisInterface


def classFactory(iface: QgisInterface):
    # The installed package goes on sys.path before any module imports it
    from .core import install

    install.activate()

    from .plugin import TopologyRules

    return TopologyRules(iface)
