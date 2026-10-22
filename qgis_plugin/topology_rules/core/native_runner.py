from __future__ import annotations

import typing as t
from pathlib import Path

from qgis.core import (
    Qgis,
    QgsCoordinateReferenceSystem,
    QgsCoordinateTransform,
    QgsFeature,
    QgsFeatureRequest,
    QgsGeometry,
    QgsProcessingContext,
    QgsProcessingFeedback,
    QgsVectorFileWriter,
)

from ..exceptions import RunError
from ..utils import QgsCsException
from .inputs import features, moves
from .invalid import MULTI, error_fields
from .layers import Input, wkb_kind

if t.TYPE_CHECKING:
    from .checks import Job, Outcome

try:
    from topology_checker import (  # pyright: ignore[reportMissingImports]
        Rule,
        Topology,
    )
    from topology_checker._enums import (  # pyright: ignore[reportMissingImports]
        _GROUPS,
    )
except Exception:
    Rule = None  # type: ignore[assignment,misc]
    Topology = None  # type: ignore[assignment,misc]
    _GROUPS = {}  # type: ignore[assignment]

try:
    import shapely
except Exception:
    shapely = None


def resolve_rule(rule_name: str) -> Rule:
    """Finds the Rule enum for a full or partial rule name."""
    if Rule is None:
        raise RunError('topology_checker is not available')
    for rule, names in _GROUPS.items():
        if rule_name in names:
            return rule
    suffix = rule_name.split('/')[-1]
    try:
        return Rule(suffix)
    except (ValueError, TypeError):
        pass
    clean = suffix.replace('-', '_').upper()
    if hasattr(Rule, clean):
        return getattr(Rule, clean)
    raise ValueError(f'Unknown rule: {rule_name}')


def decompose_geom(geom: t.Any) -> list[t.Any]:
    """Decomposes GeometryCollections into flat concrete geometries."""
    if getattr(geom, 'geom_type', None) == 'GeometryCollection':
        parts: list[t.Any] = []
        for child in getattr(geom, 'geoms', []):
            parts.extend(decompose_geom(child))
        return parts
    return [geom]


def run_native_jobs(
    jobs: t.Sequence[Job],
    tolerance: float | None,
    output: str,
    context: QgsProcessingContext,
    feedback: QgsProcessingFeedback,
) -> Outcome:
    """Executes validation in-memory via topology_checker.Topology."""
    from .checks import Outcome

    if not jobs:
        return Outcome(
            counts=[],
            ids={},
            layers={},
            invalid=[],
            crs=QgsCoordinateReferenceSystem(),
        )

    if Topology is None or Rule is None:
        raise RunError('topology_checker is not available')

    topology = Topology(tolerance=tolerance if tolerance is not None else 0.0)
    target = jobs[0].layer.source.sourceCrs()

    unique_inputs: dict[object, Input] = {}
    for job in jobs:
        if job.layer.key not in unique_inputs:
            unique_inputs[job.layer.key] = job.layer
        if job.other is not None and job.other.key not in unique_inputs:
            unique_inputs[job.other.key] = job.other

    aliases: dict[object, str] = {}
    counts_by_name: dict[str, int] = {}
    for key, inp in unique_inputs.items():
        base = inp.name or 'layer'
        count = counts_by_name.get(base, 0) + 1
        counts_by_name[base] = count
        aliases[key] = base if count == 1 else f'{base}_{count}'

    layer_ids: dict[object, t.Any] = {}
    for key, inp in unique_inputs.items():
        alias = aliases[key]
        crs = inp.source.sourceCrs()
        transform = None
        if target.isValid() and crs.isValid() and moves(crs, target):
            feedback.pushInfo(
                f'Reprojecting {inp.name} from {crs.authid()} to'
                f' {target.authid()}'
            )
            transform = QgsCoordinateTransform(
                crs, target, context.transformContext()
            )

        request = QgsFeatureRequest()
        request.setSubsetOfAttributes([])
        if inp.area is not None:
            request.setFilterRect(inp.area)
        elif inp.ids is not None:
            request.setFilterFids(list(inp.ids))

        pairs: list[tuple[int, bytes]] = []
        for feature in features(inp.source, request):
            if feedback.isCanceled():
                raise RunError('Canceled')
            if inp.ids is not None and feature.id() not in inp.ids:
                continue
            geom = feature.geometry()
            if geom.isNull() or geom.isEmpty():
                continue
            if transform is not None:
                try:
                    geom.transform(transform)
                except QgsCsException as error:
                    raise RunError(
                        f'Could not reproject {inp.name}: {error}'
                    ) from error
            pairs.append((feature.id(), bytes(geom.asWkb())))

        try:
            layer_id = topology.add_layer(
                [(fid, wkb) for fid, wkb in pairs], name=alias
            )
        except Exception:
            if shapely is not None:
                geoms = [shapely.from_wkb(wkb) for _, wkb in pairs]
                fids = [fid for fid, _ in pairs]
                layer_id = topology.add_layer(geoms, fids=fids, name=alias)
            else:
                raise
        layer_ids[key] = layer_id

    for job in jobs:
        rule_enum = resolve_rule(job.rule.name)
        layer1_id = layer_ids[job.layer.key]
        layer2_id = layer_ids[job.other.key] if job.other is not None else None
        topology.add_rule(rule_enum, layer1_id, other=layer2_id)

    report = topology.validate()
    counts = [len(rule_report.violations) for rule_report in report.rules]

    by_kind: dict[str, list[QgsFeature]] = {
        'point': [],
        'line': [],
        'polygon': [],
    }
    fields = error_fields()
    violation_id = 0

    for rule_report in report.rules:
        for violation in rule_report.violations:
            origin_alias = (
                topology.layers[violation.origin.layer.index].name
                if violation.origin is not None
                else None
            )
            dest_alias = (
                topology.layers[violation.destination.layer.index].name
                if violation.destination is not None
                else None
            )
            rule_name = violation.resolved_rule.value
            error_type = violation.error_type.value
            origin_fid = (
                violation.origin.fid if violation.origin is not None else None
            )
            origin_part = (
                violation.origin.part if violation.origin is not None else None
            )
            dest_fid = (
                violation.destination.fid
                if violation.destination is not None
                else None
            )
            dest_part = (
                violation.destination.part
                if violation.destination is not None
                else None
            )

            for sub_geom in decompose_geom(violation.geometry):
                qgs_geom = QgsGeometry()
                qgs_geom.fromWkb(sub_geom.wkb)
                qgs_geom.convertToMultiType()
                kind = wkb_kind(qgs_geom.wkbType())
                if kind not in by_kind:
                    continue
                feat = QgsFeature(fields)
                feat.setGeometry(qgs_geom)
                feat.setAttributes(
                    [
                        violation_id,
                        rule_name,
                        error_type,
                        origin_alias,
                        origin_fid,
                        origin_part,
                        dest_alias,
                        dest_fid,
                        dest_part,
                    ]
                )
                by_kind[kind].append(feat)
            violation_id += 1

    out_path = Path(output)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    if out_path.exists():
        out_path.unlink()

    wrote_any = False
    for kind, layer_features in by_kind.items():
        if not layer_features:
            continue
        layer_name = f'errors_{kind}'
        options = QgsVectorFileWriter.SaveVectorOptions()
        options.driverName = 'GPKG'
        options.fileEncoding = 'UTF-8'
        options.layerName = layer_name
        if out_path.exists():
            options.actionOnExistingFile = (
                QgsVectorFileWriter.ActionOnExistingFile.CreateOrOverwriteLayer
            )
        writer = QgsVectorFileWriter.create(
            str(out_path),
            fields,
            getattr(Qgis.WkbType, MULTI[kind]),
            target,
            context.transformContext(),
            options,
        )
        if (
            writer is None
            or writer.hasError() != QgsVectorFileWriter.WriterError.NoError
        ):
            message = '' if writer is None else writer.errorMessage()
            raise RunError(f'Could not write {layer_name}: {message}')
        res = writer.addFeatures(layer_features)
        ok = res[0] if isinstance(res, tuple) else bool(res)
        if not ok:
            raise RunError(f'Could not write feature to {layer_name}')
        del writer
        wrote_any = True

    if not wrote_any:
        options = QgsVectorFileWriter.SaveVectorOptions()
        options.driverName = 'GPKG'
        options.fileEncoding = 'UTF-8'
        options.layerName = 'errors_point'
        writer = QgsVectorFileWriter.create(
            str(out_path),
            fields,
            Qgis.WkbType.MultiPoint,
            target,
            context.transformContext(),
            options,
        )
        del writer

    layer_names = {
        aliases[key]: inp.layer_id
        for key, inp in unique_inputs.items()
        if inp.layer_id is not None
    }
    return Outcome(
        counts=counts,
        ids={},
        layers=layer_names,
        invalid=[],
        crs=target,
    )
