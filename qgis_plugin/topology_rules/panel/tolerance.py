from __future__ import annotations

from qgis.core import QgsVectorLayer
from qgis.gui import QgsDoubleSpinBox

from ..utils import tr
from .entries import Entry, project_layer


class ToleranceBox(QgsDoubleSpinBox):
    """A box whose lowest value, the default, is a mark and not a number: a
    step up from it goes to zero, not to a value just above the mark."""

    def stepBy(self, steps: int) -> None:
        if self.value() == self.minimum() and steps > 0:
            self.setValue((steps - 1) * self.singleStep())
        else:
            super().stepBy(steps)


def tolerance_box() -> QgsDoubleSpinBox:
    """A box for the tolerance, which starts at the default of the tool: the
    lowest value, shown as the word."""
    box = ToleranceBox()
    box.setDecimals(10)
    box.setRange(-1.0, 1e12)
    box.setSingleStep(0.001)
    box.setSpecialValueText(tr('Default'))
    box.setClearValueMode(QgsDoubleSpinBox.ClearValueMode.MinimumValue)
    box.setShowClearButton(True)
    box.setValue(box.minimum())

    def snap(value: float) -> None:
        # Below zero there is only the default: no tolerance is negative
        if box.minimum() < value < 0:
            box.setValue(box.minimum())

    box.valueChanged.connect(snap)
    return box


def show_unit(box: QgsDoubleSpinBox, first: Entry | None) -> None:
    """Distances are in the units of the first layer of the list."""
    box.setSuffix(f' {first.unit}' if first and first.unit else '')
    box.setSpecialValueText(
        default_tolerance(first and project_layer(first.layer))
    )


def default_tolerance(layer: QgsVectorLayer | None) -> str:
    """What the tool does without a tolerance, for the CRS of the layer."""
    if layer is None or not layer.crs().isValid():
        return tr('Default')
    if layer.crs().isGeographic():
        return tr('Default (1e-8°)')
    return tr('Default (1 mm)')
