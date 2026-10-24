"""The menu of the gear button: the engine that runs the checks, the
Python package and the executable that it can be, and how the errors are
drawn."""

from __future__ import annotations

import typing as t

from qgis.PyQt.QtCore import pyqtSignal
from qgis.PyQt.QtWidgets import (
    QAction,
    QActionGroup,
    QMenu,
    QStyle,
    QWidget,
)

from ..core import binary, engine, install, wheels
from ..core.engine import Engine, Package
from ..core.wheels import PACKAGE
from ..exceptions import ExecutableError, MissingError, PackageError
from ..utils import not_none, tr
from .style_menu import StyleMenu


def engine_names() -> dict[Engine, str]:
    """The engines in the order of the menu, with their names in it."""
    return {
        Engine.AUTOMATIC: tr('Automatic'),
        Engine.PACKAGE: tr('Python package'),
        Engine.EXECUTABLE: tr('Executable'),
    }


def installable() -> bool:
    """Whether the package can be installed: the plugin did not install it,
    QGIS has none that it can use, and it would load in this QGIS."""
    return (
        install.installed() is None
        and engine.package() is None
        and install.unsupported() is None
    )


def reason(error: ExecutableError) -> str:
    """Why the engine cannot run, as the panel says it: next to its buttons
    to install the package and choose the executable, what is missing is
    enough."""
    if isinstance(error, MissingError):
        return tr('topology-checker was not found.')
    return str(error)


class PanelMenu(QMenu):
    """What the engine is, what can be done about it, and the style of the
    errors. It is filled in each time it opens, as the package and the
    executable change outside it."""

    engine_chosen = pyqtSignal(object)

    def __init__(self, parent: QWidget | None = None):
        super().__init__(parent)
        self.setToolTipsVisible(True)
        self.group = QActionGroup(self)
        self.group.setExclusive(True)
        # QAction moved between modules in Qt6, so it is no type here
        self.engines: dict[Engine, t.Any] = {}
        self.install_action = QAction(tr('Install Python package…'), self)
        self.update_action = QAction(self)
        self.remove_action = QAction(tr('Remove Python package'), self)
        self.choose_action = QAction(tr('Choose executable…'), self)
        self.style_menu = StyleMenu(self)

        # The sections, in the order of the menu
        self.add_engines()
        self.addSeparator()
        self.addActions(
            [
                self.install_action,
                self.update_action,
                self.remove_action,
                self.choose_action,
            ]
        )
        self.addSeparator()
        self.addMenu(self.style_menu)

        self.group.triggered.connect(self.engine_triggered)
        self.aboutToShow.connect(self.fill)

    def heading(self, text: str) -> None:
        """A title over the items that follow. A style that draws sections as
        plain lines gets a title that cannot be clicked instead."""
        style = not_none(self.style())
        hint = QStyle.StyleHint.SH_Menu_SupportsSections
        if style.styleHint(hint, None, self):
            self.addSection(text)
            return
        title = not_none(self.addAction(text))
        title.setEnabled(False)

    def add_engines(self) -> None:
        self.heading(tr('Engine'))
        for choice, name in engine_names().items():
            action = QAction(name, self)
            action.setCheckable(True)
            action.setData(choice.value)
            self.group.addAction(action)
            self.addAction(action)
            self.engines[choice] = action
        self.engines[Engine.AUTOMATIC].setToolTip(
            tr('The Python package where QGIS can use it, else the executable')
        )

    def fill(self) -> None:
        """Shows the engine that is chosen, and what can be done now."""
        self.engines[engine.chosen()].setChecked(True)
        found: Package | None = None
        try:
            found = engine.find_package()
        except PackageError as error:
            self.unusable(Engine.PACKAGE, str(error), error.brief)
        else:
            self.usable(
                Engine.PACKAGE,
                tr('{package} {version} in {folder}').format(
                    package=PACKAGE,
                    version=found.version,
                    folder=found.location,
                ),
            )
        try:
            path = binary.find_executable()
        except ExecutableError as error:
            self.unusable(Engine.EXECUTABLE, reason(error), tr('not found'))
        else:
            self.usable(Engine.EXECUTABLE, str(path))

        version = install.installed()
        self.install_action.setVisible(version is None and found is None)
        refusal = wheels.refusal()
        if refusal is None:
            self.install_action.setText(tr('Install Python package…'))
            self.offer(self.install_action, None)
        else:
            self.refuse(
                self.install_action,
                tr('Install Python package'),
                str(refusal),
                refusal.brief,
            )
        self.update_action.setVisible(install.outdated())
        self.update_action.setText(
            tr('Update Python package to {version}…').format(
                version=install.wanted()
            )
        )
        self.remove_action.setVisible(version is not None)

    def usable(self, choice: Engine, tip: str) -> None:
        action = self.engines[choice]
        action.setText(engine_names()[choice])
        self.offer(action, None, tip)

    def unusable(self, choice: Engine, reason: str, brief: str) -> None:
        self.refuse(self.engines[choice], engine_names()[choice], reason, brief)

    def refuse(self, action: t.Any, name: str, reason: str, brief: str) -> None:
        """Greys out the item, with why in a few words after its name: a tip
        shows only to a pointer that rests on the item."""
        action.setText(
            tr('{name} ({why})').format(
                name=name, why=brief or tr('unavailable')
            )
        )
        self.offer(action, reason)

    def offer(self, action: t.Any, reason: str | None, tip: str = '') -> None:
        """Lets the item be chosen, with a tip, or says why it cannot be."""
        action.setEnabled(reason is None)
        action.setToolTip(reason or tip)
        action.setStatusTip(reason or tip)

    def engine_triggered(self, action: t.Any) -> None:
        """Says that the user chose another engine. The one that is chosen
        already changes nothing."""
        chosen = Engine(action.data())
        if chosen is not engine.chosen():
            self.engine_chosen.emit(chosen)
