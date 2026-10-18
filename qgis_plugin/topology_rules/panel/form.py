from __future__ import annotations

import contextlib
import typing as t

from qgis.core import Qgis, QgsProject, QgsVectorLayer
from qgis.gui import QgsMapLayerComboBox
from qgis.PyQt.QtCore import QCoreApplication, Qt
from qgis.PyQt.QtWidgets import (
    QComboBox,
    QFormLayout,
    QLabel,
    QPushButton,
    QStackedLayout,
    QVBoxLayout,
    QWidget,
)

from ..core.binary import Rule
from ..core.layers import wkb_kind
from ..utils import not_none
from .entries import Entry, unit_of
from .icons import decorate


def tr(text: str) -> str:
    return QCoreApplication.translate('TopologyRules', text)


def layer_filter(kind: str) -> t.Any:
    filters = Qgis.LayerFilter
    return {
        'point': filters.PointLayer,
        'line': filters.LineLayer,
        'polygon': filters.PolygonLayer,
    }.get(kind, filters.HasGeometry)


class RuleForm(QWidget):
    """Picks a layer, then one of the rules that fit it, and the second
    layer of the rules that take two."""

    def __init__(self, parent: QWidget | None = None):
        super().__init__(parent)
        self.rules: tuple[Rule, ...] = ()
        self.hint = QLabel(tr('Add a layer with geometries to the project.'))
        self.hint.setWordWrap(True)
        self.hint.setAlignment(Qt.AlignmentFlag.AlignTop)
        self.input_box = QgsMapLayerComboBox()
        self.input_box.setFilters(Qgis.LayerFilter.HasGeometry)
        self.rule_box = QComboBox()
        self.description = QLabel()
        self.description.setWordWrap(True)
        self.description.setAlignment(Qt.AlignmentFlag.AlignTop)
        # The hint and the description take the same place, which has room
        # for a few lines, so that the form does not move when one replaces
        # the other
        self.note = QWidget()
        self.note.setMinimumHeight(
            4 * self.note.fontMetrics().lineSpacing() + 8
        )
        for label in (self.hint, self.description):
            font = label.font()
            font.setPointSizeF(font.pointSizeF() * 0.9)
            label.setFont(font)
            label.setStyleSheet('color: palette(mid);')
        notes = QStackedLayout(self.note)
        notes.setContentsMargins(0, 0, 0, 0)
        notes.addWidget(self.hint)
        notes.addWidget(self.description)
        self.notes = notes
        self.other_label = QLabel(tr('Other layer'))
        self.other_box = QgsMapLayerComboBox()
        self.add_button = QPushButton(tr('Add rule'))
        decorate(self.add_button, 'add', tr('Add the rule to the list'))

        form = QFormLayout()
        form.addRow(tr('Layer'), self.input_box)
        form.addRow(tr('Rule'), self.rule_box)
        form.addRow(self.other_label, self.other_box)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(2)
        self.fields = QWidget()
        self.fields.setLayout(form)
        layout.addWidget(self.fields)
        # The note is last, where the room it keeps looks like the room
        # above a button
        layout.addWidget(self.note)
        layout.addWidget(self.add_button)

        self.input_box.layerChanged.connect(self.list_rules)
        self.rule_box.currentIndexChanged.connect(self.show_rule)
        project = not_none(QgsProject.instance())
        project.layersAdded.connect(self.list_rules)
        project.layersRemoved.connect(self.list_rules)
        self.list_rules()

    def dispose(self) -> None:
        project = not_none(QgsProject.instance())
        for signal in (project.layersAdded, project.layersRemoved):
            with contextlib.suppress(TypeError):
                signal.disconnect(self.list_rules)

    def set_rules(self, rules: tuple[Rule, ...]) -> None:
        self.rules = rules
        self.list_rules()

    def layer(self) -> QgsVectorLayer | None:
        layer = self.input_box.currentLayer()
        return layer if isinstance(layer, QgsVectorLayer) else None

    def entry(self) -> Entry | str:
        """The rule and layers of the form as an entry of the list, or what
        is missing."""
        rule = self.current_rule()
        layer = self.layer()
        other = self.other_box.currentLayer() if rule and rule.other else None
        if rule is None or layer is None:
            return tr('Choose a layer and a rule.')
        if rule.other is not None and not isinstance(other, QgsVectorLayer):
            return tr('Choose the other layer.')
        return Entry(
            rule,
            layer.id(),
            layer.name(),
            unit_of(layer),
            other.id() if isinstance(other, QgsVectorLayer) else None,
            other.name() if isinstance(other, QgsVectorLayer) else '',
        )

    def current_rule(self) -> Rule | None:
        return t.cast('Rule | None', self.rule_box.currentData())

    def list_rules(self) -> None:
        """Lists the rules that the layer can have, those of one layer first,
        and keeps the rule if it still fits."""
        layer = self.layer()
        self.notes.setCurrentWidget(
            self.hint if layer is None else self.description
        )
        self.fields.setEnabled(layer is not None)
        kept = self.current_rule()
        fitting = (
            []
            if layer is None
            else [r for r in self.rules if r.fits(wkb_kind(layer.wkbType()))]
        )
        # The rules of the kind of the layer, then those for any geometry,
        # then those with a second layer
        fitting.sort(
            key=lambda rule: (
                rule.other is not None,
                rule.layer == 'any',
                rule.name,
            )
        )
        self.rule_box.blockSignals(True)
        self.rule_box.clear()
        for rule in fitting:
            self.rule_box.addItem(rule.short_name, rule)
        if kept in fitting:
            self.rule_box.setCurrentIndex(fitting.index(kept))
        self.rule_box.blockSignals(False)
        self.show_rule()

    def show_rule(self) -> None:
        rule = self.current_rule()
        self.description.setText('' if rule is None else rule.description)
        has_other = rule is not None and rule.other is not None
        self.other_label.setVisible(has_other)
        self.other_box.setVisible(has_other)
        if rule is not None and rule.other is not None:
            self.other_box.setFilters(layer_filter(rule.other))
        self.add_button.setEnabled(
            rule is not None and self.layer() is not None
        )

    def choose(self, rule: Rule) -> None:
        """Shows the rule in the form if the layer can have it."""
        index = self.rule_box.findData(rule)
        if index >= 0:
            self.rule_box.setCurrentIndex(index)
