from __future__ import annotations

import contextlib
from pathlib import Path

from qgis.core import (
    Qgis,
    QgsApplication,
    QgsProcessingAlgRunnerTask,
    QgsProcessingContext,
    QgsProject,
    QgsTask,
)
from qgis.gui import QgisInterface, QgsDockWidget
from qgis.PyQt import sip
from qgis.PyQt.QtCore import Qt, QUrl
from qgis.PyQt.QtGui import QDesktopServices
from qgis.PyQt.QtWidgets import (
    QHBoxLayout,
    QProgressBar,
    QPushButton,
    QTabWidget,
    QToolButton,
    QVBoxLayout,
    QWidget,
)

from ..algorithms.base import DOCS
from ..core import engine, install
from ..core.binary import Rule
from ..core.engine import Engine
from ..core.style import ColourBy, Legend
from ..enums import PROVIDER_ID
from ..exceptions import ExecutableError
from ..utils import not_none, tr
from . import controller, exchange, startup
from .entries import Entry
from .groups import restyle_errors
from .icons import icon
from .installer import InstallDialog
from .menu import PanelMenu, installable, reason, relist
from .messages import RunFeedback, push_summary
from .narrow import shrink_buttons, shrink_tabs
from .offer import stop_offering
from .results import ResultsPanel
from .rules_page import RulesPage
from .setup import SetupPage
from .state import ProjectState
from .style_menu import StyleMenu
from .tolerance import show_unit

# The part of the documentation about the plugin
HELP_URL = f'{DOCS}install/#qgis'


class Dock(QgsDockWidget):
    """Collects rules to run on layers, then lists the errors they find."""

    def __init__(self, iface: QgisInterface, parent: QWidget | None = None):
        super().__init__(tr('Topology Rules'), parent)
        self.setObjectName('TopologyRulesDock')
        self.iface = iface
        self.available = False
        # The executable that is being asked for its rules, and the task
        self.probing: tuple[str, int] | None = None
        self.probe_task: QgsTask | None = None
        self.run: controller.Run | None = None
        self.task: QgsProcessingAlgRunnerTask | None = None
        # The task and the objects it uses live until it finishes
        self.context: QgsProcessingContext | None = None
        self.feedback: RunFeedback | None = None
        self.runs = 0
        # Layers without selected features in the last run that wanted them
        self.unselected: list[str] = []
        # The group of errors that each rule of the list had in the last run
        self.run_keys: dict[tuple[str, str, str | None], str] = {}
        # The install of the package that is open
        self.installer: InstallDialog | None = None
        # The groups that the errors of the last run are drawn in
        self.legends: dict[ColourBy, Legend] = {}

        self.setup_page = SetupPage()
        self.status = self.setup_page.status
        self.browse_button = self.setup_page.browse_button
        self.install_button = self.setup_page.install_button
        self.rules_page = RulesPage()
        self.offer = self.rules_page.offer
        self.form = self.rules_page.form
        self.input_box = self.form.input_box
        self.rule_box = self.form.rule_box
        self.description = self.form.description
        self.other_label = self.form.other_label
        self.other_box = self.form.other_box
        self.add_button = self.form.add_button
        self.hint = self.form.hint
        self.rule_list = self.rules_page.rule_list
        self.rules_tree = self.rule_list.tree
        self.rules_placeholder = self.rule_list.placeholder
        self.remove_button = self.rule_list.remove_button
        self.clear_button = self.rule_list.clear_button
        self.import_button = self.rules_page.import_button
        self.export_button = self.rules_page.export_button
        self.selected_only = self.rules_page.selected_only
        self.extent_only = self.rules_page.extent_only
        self.tolerance = self.rules_page.tolerance
        self.results = ResultsPanel(iface)
        self.results_view = self.results.view
        self.results_placeholder = self.results.placeholder
        self.show_ignored = self.results.show_ignored
        self.select_button = self.results.select_button
        self.run_button = QPushButton(tr('Run'))
        self.help_button = QPushButton()
        self.help_button.setIcon(icon('help'))
        self.help_button.setToolTip(tr('Open the documentation'))
        self.help_button.setFlat(True)
        self.menu = PanelMenu(self)
        self.menu_button = QToolButton()
        self.menu_button.setIcon(icon('menu'))
        self.menu_button.setToolTip(tr('Settings'))
        self.menu_button.setAutoRaise(True)
        self.menu_button.setPopupMode(
            QToolButton.ToolButtonPopupMode.InstantPopup
        )
        self.menu_button.setMenu(self.menu)
        self.style_menu = StyleMenu(self)
        self.progress = QProgressBar()
        self.progress.setRange(0, 0)
        self.progress.hide()

        self.tabs = QTabWidget()
        shrink_tabs(self.tabs)
        shrink_buttons(self.run_button)
        self.tabs.addTab(self.rules_page, tr('Rules'))
        self.tabs.addTab(self.results, tr('Errors'))
        layout = QVBoxLayout()
        layout.setContentsMargins(0, 2, 0, 2)
        layout.addWidget(self.setup_page)
        layout.addWidget(self.tabs, 1)
        layout.addWidget(self.progress)
        bottom = QHBoxLayout()
        bottom.addWidget(self.run_button, 1)
        bottom.addWidget(self.menu_button)
        bottom.addWidget(self.help_button)
        layout.addLayout(bottom)
        body = QWidget()
        body.setLayout(layout)
        self.setWidget(body)

        self.state = ProjectState(self)
        self.browse_button.clicked.connect(self.choose_executable)
        self.install_button.clicked.connect(self.install_package)
        self.offer.install_clicked.connect(self.install_package)
        self.menu.engine_chosen.connect(self.choose_engine)
        self.menu.install_action.triggered.connect(self.install_package)
        self.menu.update_action.triggered.connect(self.update_package)
        self.menu.remove_action.triggered.connect(self.remove_package)
        self.menu.choose_action.triggered.connect(self.choose_executable)
        self.import_button.clicked.connect(self.choose_import)
        self.export_button.clicked.connect(self.choose_export)
        self.add_button.clicked.connect(self.add_rule)
        self.run_button.clicked.connect(self.run_or_cancel)
        self.help_button.clicked.connect(self.open_help)
        self.rule_list.changed.connect(self.update_buttons)
        self.rule_list.changed.connect(self.state.save)
        self.tolerance.valueChanged.connect(self.state.save)
        self.selected_only.toggled.connect(self.state.save)
        self.extent_only.toggled.connect(self.state.save)
        self.rule_list.opened.connect(self.open_errors)
        self.results.ignored_changed.connect(self.errors_ignored)
        self.style_menu.changed.connect(self.restyle)
        self.visibilityChanged.connect(self.refresh)
        project = not_none(QgsProject.instance())
        project.layersRemoved.connect(self.layers_removed)
        project.cleared.connect(self.project_cleared)
        project.readProject.connect(self.state.restore)
        self.update_buttons()

    @property
    def entries(self) -> list[Entry]:
        return self.rule_list.entries

    def refresh(self, visible: bool = True) -> None:
        """Lists the rules of the engine, or says why there are none."""
        if visible:
            startup.refresh(self)

    def rules_known(self, rules: tuple[Rule, ...]) -> None:
        self.available = True
        self.load_algorithms()
        self.form.set_rules(rules)
        self.state.fill()
        self.offer.update_offer()
        self.update_buttons()

    def load_algorithms(self) -> None:
        """Has the provider list its algorithms, which it does by itself
        when the executable answers, but maybe after the panel was told."""
        registry = not_none(QgsApplication.processingRegistry())
        provider = registry.providerById(PROVIDER_ID)
        if provider is not None and not provider.algorithms():
            provider.refreshAlgorithms()

    def rules_failed(self, error: ExecutableError) -> None:
        self.available = False
        self.form.set_rules(())
        self.setup_page.explain(reason(error), installable())
        self.update_buttons()

    def current_rule(self) -> Rule | None:
        return self.form.current_rule()

    def show_rule(self) -> None:
        self.form.show_rule()

    def reload(self, chosen: Engine | None = None) -> None:
        """Has the toolbox and the panel list the rules of the engine, after
        it or what it runs changed, and first chooses the engine if one is
        given."""
        relist(chosen)
        self.refresh()

    def choose_executable(self) -> None:
        """Lets the user point to the executable, then looks for the rules."""
        if self.setup_page.choose():
            self.reload()

    def choose_engine(self, chosen: Engine) -> None:
        if chosen is not engine.chosen():
            self.reload(chosen)

    def install_package(self) -> None:
        self.open_installer(update=False)

    def update_package(self) -> None:
        self.open_installer(update=True)

    def open_installer(self, update: bool) -> None:
        """Shows the install of the package, or the one that is open."""
        if self.installer is not None and not sip.isdeleted(self.installer):
            self.installer.show()
            self.installer.raise_()
            self.installer.activateWindow()
            return
        dialog = InstallDialog(self, update)
        dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        dialog.installed.connect(self.package_installed)
        self.installer = dialog
        dialog.show()

    def package_installed(self) -> None:
        stop_offering()
        self.offer.hide()
        # The dialog had the toolbox list the rules
        self.refresh()

    def remove_package(self) -> None:
        """Deletes the package that the plugin installed. One that is loaded
        stays until QGIS restarts."""
        install.remove()
        text = tr('Removed the Python package.')
        if install.restart_needed():
            text = tr('Removed the Python package. Restart QGIS to unload it.')
        push_summary(
            not_none(self.iface.messageBar()), text, Qgis.MessageLevel.Success
        )
        # Else the panel would say that the package is missing
        self.reload(
            Engine.AUTOMATIC if engine.chosen() is Engine.PACKAGE else None
        )

    def warn(self, text: str) -> None:
        not_none(self.iface.messageBar()).pushWarning(
            tr('Topology Rules'), text
        )

    def runnable(self) -> list[Entry]:
        return [entry for entry in self.entries if entry.checked]

    def update_buttons(self) -> None:
        count = len(self.runnable())
        self.setup_page.setVisible(not self.available)
        for widget in (self.tabs, self.run_button):
            widget.setVisible(self.available)
        self.add_button.setEnabled(
            self.available and self.form.current_rule() is not None
        )
        controller.show_run_button(self, count)
        # The run takes the CRS of its first rule, whose unit this is
        show_unit(self.tolerance, next(iter(self.runnable()), None))
        self.rules_page.update_title()

    def add_rule(self) -> None:
        """Adds the rule and layers of the form to the list."""
        entry = self.form.entry()
        if isinstance(entry, str):
            self.warn(entry)
        elif not self.rule_list.add(entry):
            self.warn(tr('The rule is already in the list.'))

    def remove_rules(self) -> None:
        if self.task is None:
            self.rule_list.remove_selected()

    def clear_rules(self) -> None:
        if self.task is None:
            self.rule_list.clear()

    def open_errors(self, entry: Entry) -> None:
        """Shows the errors of a rule of the list."""
        key = self.run_keys.get(entry.key)
        if key is not None:
            self.results.select_key(key)
        self.tabs.setCurrentIndex(controller.ERRORS_TAB)

    def open_help(self) -> None:
        QDesktopServices.openUrl(QUrl(HELP_URL))

    def run_or_cancel(self) -> None:
        if self.task is None:
            self.run_rules()
        else:
            self.cancel()

    def run_rules(self) -> None:
        controller.start(self)

    def tolerance_value(self) -> float | None:
        """The tolerance that was set, or none for the default."""
        value = self.tolerance.value()
        return value if value >= 0 else None

    def show_invalid(self) -> None:
        self.results.select_key('')
        self.tabs.setCurrentIndex(controller.ERRORS_TAB)

    def export_rules(self, path: str | Path) -> None:
        exchange.export_rules(self, Path(path))

    def import_rules(self, path: str | Path) -> None:
        exchange.import_rules(self, Path(path))

    def choose_export(self) -> None:
        exchange.ask_export(self)

    def choose_import(self) -> None:
        exchange.ask_import(self)

    def set_error_count(self, count: int) -> None:
        self.tabs.setTabText(
            controller.ERRORS_TAB,
            tr('Errors ({count})').format(count=count)
            if count
            else tr('Errors'),
        )

    def restyle(self) -> None:
        """Draws the errors in the style that the user chose."""
        restyle_errors(self.legends)

    def errors_ignored(self) -> None:
        """Counts the errors that are left, and keeps the ignored ones."""
        controller.recount(self)
        self.set_error_count(len(self.results.active()))
        self.state.save()

    def layers_removed(self, layer_ids: list[str]) -> None:
        """Drops the errors of layers that left the project."""
        self.results.drop_layers(layer_ids)
        self.set_error_count(len(self.results.active()))
        self.rule_list.refresh_marks()

    def project_cleared(self) -> None:
        """Starts over: the rules were about layers of the old project, and
        a run for them has nothing to show."""
        controller.abandon(self)
        self.state.pending = None
        self.results.ignored.clear()
        self.results.clear()
        # Error layers that the project has are drawn by what they store
        self.legends = {}
        self.set_error_count(0)
        with self.state.quiet():
            self.rule_list.clear()
            self.tolerance.setValue(self.tolerance.minimum())
            self.selected_only.setChecked(False)
            self.extent_only.setChecked(False)

    def cancel(self) -> None:
        if self.feedback is not None:
            self.feedback.cancel()

    def dispose(self) -> None:
        """Stops a run and lets go of the project, before the panel goes."""
        controller.abandon(self)
        project = not_none(QgsProject.instance())
        for signal, slot in (
            (project.layersRemoved, self.layers_removed),
            (project.cleared, self.project_cleared),
            (project.readProject, self.state.restore),
        ):
            with contextlib.suppress(TypeError):
                signal.disconnect(slot)
        self.form.dispose()
        self.results.focus.clear()
        if self.installer is not None and not sip.isdeleted(self.installer):
            self.installer.stop()
            self.installer.deleteLater()
        self.installer = None
