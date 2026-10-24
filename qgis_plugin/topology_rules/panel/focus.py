from __future__ import annotations

from qgis.core import (
    QgsCoordinateReferenceSystem,
    QgsCoordinateTransform,
    QgsGeometry,
    QgsProject,
    QgsRectangle,
    QgsVectorLayer,
)
from qgis.gui import QgisInterface, QgsHighlight, QgsMapCanvas
from qgis.PyQt import sip
from qgis.PyQt.QtGui import QColor

from ..core.style import RED
from ..utils import not_none

# How much room the view keeps around an error, as a multiple of its size
MARGIN = 1.5
# The scale (as 1:n) that a point is shown at when the map is further out
POINT_SCALE = 2000


class Focus:
    """Shows one error on the map: zooms to it and marks it until the next
    one is shown."""

    def __init__(self, iface: QgisInterface):
        self.iface = iface
        self.highlight: QgsHighlight | None = None

    def canvas(self) -> QgsMapCanvas:
        return not_none(self.iface.mapCanvas())

    def clear(self) -> None:
        if self.highlight is not None and not sip.isdeleted(self.highlight):
            self.highlight.hide()
            sip.delete(self.highlight)
        self.highlight = None

    def show(self, layer: QgsVectorLayer, fid: int) -> None:
        """Zooms to the feature, flashes it and leaves it marked."""
        self.clear()
        feature = layer.getFeature(fid)
        geometry = feature.geometry()
        if not feature.isValid() or geometry.isNull():
            return
        canvas = self.canvas()
        self.zoom(canvas, layer.crs(), geometry)
        self.mark(QgsHighlight(canvas, geometry, layer))
        self.flash(canvas, geometry, layer.crs())

    def show_geometry(
        self, geometry: QgsGeometry, crs: QgsCoordinateReferenceSystem
    ) -> None:
        """The same for an error that is not in a layer yet."""
        self.clear()
        if geometry.isNull():
            return
        canvas = self.canvas()
        self.zoom(canvas, crs, geometry)
        self.mark(QgsHighlight(canvas, self.moved(canvas, crs, geometry), None))
        self.flash(canvas, geometry, crs)

    def mark(self, marker: QgsHighlight) -> None:
        marker.setColor(QColor(RED))
        marker.setFillColor(QColor(RED))
        marker.setWidth(3)
        marker.show()
        self.highlight = marker

    def flash(
        self,
        canvas: QgsMapCanvas,
        geometry: QgsGeometry,
        crs: QgsCoordinateReferenceSystem,
    ) -> None:
        canvas.flashGeometries(
            [geometry], crs, QColor(255, 255, 255, 200), QColor(RED)
        )

    def moved(
        self,
        canvas: QgsMapCanvas,
        crs: QgsCoordinateReferenceSystem,
        geometry: QgsGeometry,
    ) -> QgsGeometry:
        """The geometry in the CRS of the map."""
        moved = QgsGeometry(geometry)
        moved.transform(
            QgsCoordinateTransform(
                crs,
                canvas.mapSettings().destinationCrs(),
                not_none(QgsProject.instance()),
            )
        )
        return moved

    def zoom(
        self,
        canvas: QgsMapCanvas,
        crs: QgsCoordinateReferenceSystem,
        geometry: QgsGeometry,
    ) -> None:
        """Keeps the error in view with room around it. A point is only
        centred, as there is no size to keep the room around."""
        moved = self.moved(canvas, crs, geometry)
        box = moved.boundingBox()
        if box.width() == 0 and box.height() == 0:
            canvas.setCenter(box.center())
            # Close enough to see what is around it, never closer than the
            # map already is
            if canvas.scale() > POINT_SCALE:
                canvas.zoomScale(POINT_SCALE)
            canvas.refresh()
            return
        side = max(box.width(), box.height())
        # A flat line has no height to scale
        wanted = QgsRectangle(
            box.center().x() - side / 2,
            box.center().y() - side / 2,
            box.center().x() + side / 2,
            box.center().y() + side / 2,
        )
        wanted.scale(MARGIN)
        canvas.setExtent(wanted, True)
        canvas.refresh()
