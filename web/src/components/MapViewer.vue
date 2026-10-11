<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref, watch } from "vue";
import Map from "ol/Map";
import View from "ol/View";
import Feature from "ol/Feature";
import GeoJSON from "ol/format/GeoJSON";
import Projection from "ol/proj/Projection";
import VectorLayer from "ol/layer/Vector";
import TileLayer from "ol/layer/Tile";
import VectorSource from "ol/source/Vector";
import OSM from "ol/source/OSM";
import { Circle, Fill, Stroke, Style } from "ol/style";
import { extend, createEmpty, isEmpty } from "ol/extent";
import { useWorkspace } from "../store";
import { request } from "../api";
import type { Viewport } from "../types";
const store = useWorkspace();
const target = ref<HTMLDivElement>();
const canvas = ref<HTMLCanvasElement>();
const truncated = ref(false);
const loadError = ref("");
const coordinates = ref("");
const loading = ref(false);
let map: Map | undefined;
let generation = 0;
let abort: AbortController | undefined;
let observer: ResizeObserver | undefined;
const sources = new globalThis.Map<string, VectorSource>();
const layers = new globalThis.Map<string, VectorLayer<VectorSource>>();
const highlight = new VectorSource();
const format = new GeoJSON();
const local = new Projection({ code: "WORKING", units: "m", global: false });
const selectedStyle = new Style({
  stroke: new Stroke({ color: "#ed633e", width: 4 }),
  fill: new Fill({ color: "#ed633e33" }),
  image: new Circle({
    radius: 7,
    fill: new Fill({ color: "#ed633e" }),
    stroke: new Stroke({ color: "white", width: 2 }),
  }),
});
const contributorStyle = new Style({
  stroke: new Stroke({ color: "#eeb23f", width: 5 }),
  fill: new Fill({ color: "#eeb23f22" }),
  image: new Circle({ radius: 9, fill: new Fill({ color: "#eeb23f" }) }),
});
function style(color: string) {
  return new Style({
    stroke: new Stroke({ color, width: 2 }),
    fill: new Fill({ color: `${color}20` }),
    image: new Circle({
      radius: 5,
      fill: new Fill({ color }),
      stroke: new Stroke({ color: "#fff", width: 1.5 }),
    }),
  });
}
function build() {
  if (!target.value || !store.ws) return;
  generation++;
  abort?.abort();
  const priorCenter = map?.getView().getCenter();
  map?.dispose();
  sources.clear();
  layers.clear();
  highlight.clear();
  const config = store.ws.config;
  const view = new View({
    projection: config.view.basemap ? "EPSG:3857" : local,
    center: config.view.center ?? priorCenter ?? [0, 0],
    resolution: config.view.resolution ?? 1,
    minResolution: 1e-12,
    maxResolution: 1e10,
    constrainResolution: false,
    enableRotation: false,
  });
  map = new Map({ target: target.value, view, layers: [] });
  if (config.view.basemap) map.addLayer(new TileLayer({ source: new OSM() }));
  for (const meta of store.ws.layers) {
    const settings = config.layers.find((l) => l.id === meta.id)!;
    if (!settings.selected) continue;
    const source = new VectorSource({ wrapX: false });
    const layer = new VectorLayer({
      source,
      style: style(settings.color),
      visible: settings.visible,
    });
    sources.set(meta.id, source);
    layers.set(meta.id, layer);
    map.addLayer(layer);
  }
  map.addLayer(
    new VectorLayer({
      source: highlight,
      style: (f) => (f.get("contributor") ? contributorStyle : selectedStyle),
      zIndex: 20,
    }),
  );
  map.on("moveend", () => {
    drawGrid();
    void load();
    const v = map!.getView();
    if (store.draft) {
      store.draft.view.center = v.getCenter() as [number, number];
      store.draft.view.resolution = v.getResolution()!;
    }
  });
  map.on("postrender", drawGrid);
  map.on("pointermove", (e) => {
    coordinates.value = `${config.view.basemap ? "Display" : "Working"} X ${e.coordinate[0]!.toLocaleString(undefined, { maximumFractionDigits: 5 })}   Y ${e.coordinate[1]!.toLocaleString(undefined, { maximumFractionDigits: 5 })}`;
  });
  map.on("singleclick", (e) => {
    map!.forEachFeatureAtPixel(
      e.pixel,
      (f) => {
        if (f.get("_layer")) {
          void store.inspect(f.get("_layer"), String(f.getId()));
          return true;
        }
        return false;
      },
      { hitTolerance: 5 },
    );
  });
  if (!config.view.center) fit();
  drawGrid();
  void load();
}
function fit() {
  if (!map || !store.ws) return;
  const extent = createEmpty();
  for (const meta of store.ws.layers) {
    if (store.fitLayer && meta.id !== store.fitLayer) continue;
    const settings = store.ws.config.layers.find((l) => l.id === meta.id);
    if (!settings?.selected || !settings.visible) continue;
    const b = store.ws.config.view.basemap ? meta.display_extent : meta.extent;
    if (b) extend(extent, b);
  }
  if (!isEmpty(extent)) {
    const width = Math.max(
      extent[2]! - extent[0]!,
      extent[3]! - extent[1]!,
      store.ws.config.tolerance * 10,
      1e-5,
    );
    const padding = width * 0.12;
    map
      .getView()
      .fit(
        [
          extent[0]! - padding,
          extent[1]! - padding,
          extent[2]! + padding,
          extent[3]! + padding,
        ],
        { padding: [48, 48, 48, 48] },
      );
  }
}
async function load() {
  if (!map || !store.ws || !map.getSize()) return;
  abort?.abort();
  abort = new AbortController();
  const version = ++generation;
  loading.value = true;
  loadError.value = "";
  const bounds = map.getView().calculateExtent(map.getSize());
  const id = store.ws.id;
  let incomplete = false;
  try {
    await Promise.all(
      [...sources].map(async ([layer, source]) => {
        if (!layers.get(layer)?.getVisible()) {
          source.clear();
          return;
        }
        const result = await request<Viewport>(
          `/workspaces/${id}/layers/${layer}/features?bbox=${bounds.join(",")}&display=${store.ws!.config.view.basemap}`,
          { signal: abort!.signal },
        );
        if (version !== generation) return;
        incomplete ||= result.incomplete;
        const features = format.readFeatures(result);
        for (const f of features) f.set("_layer", layer);
        source.clear();
        source.addFeatures(features);
      }),
    );
    if (version === generation) truncated.value = incomplete;
  } catch (e) {
    if (
      version === generation &&
      !(e instanceof DOMException && e.name === "AbortError")
    )
      loadError.value = String(e);
  } finally {
    if (version === generation) loading.value = false;
  }
}
function drawGrid() {
  if (!canvas.value || !map || !target.value) return;
  const ratio = window.devicePixelRatio || 1;
  const w = target.value.clientWidth,
    h = target.value.clientHeight;
  if (canvas.value.width !== w * ratio || canvas.value.height !== h * ratio) {
    canvas.value.width = w * ratio;
    canvas.value.height = h * ratio;
  }
  const context = canvas.value.getContext("2d")!;
  context.setTransform(ratio, 0, 0, ratio, 0, 0);
  context.clearRect(0, 0, w, h);
  if (store.ws?.config.view.basemap) return;
  const resolution = map.getView().getResolution() ?? 1;
  const rough = resolution * 100;
  const power = 10 ** Math.floor(Math.log10(rough));
  const step = [1, 2, 5, 10].find((n) => n * power >= rough)! * power;
  const extent = map.getView().calculateExtent(map.getSize());
  context.strokeStyle = "#ccd6d066";
  context.fillStyle = "#82928b";
  context.font = "10px ui-monospace, monospace";
  context.lineWidth = 1;
  for (
    let x = Math.ceil(extent[0]! / step) * step, n = 0;
    x <= extent[2]! && n < 100;
    x += step, n++
  ) {
    const p = map.getPixelFromCoordinate([x, extent[1]!]);
    if (!p) continue;
    context.beginPath();
    context.moveTo(p[0]!, 0);
    context.lineTo(p[0]!, h);
    context.stroke();
    context.fillText(
      Number(x.toPrecision(10)).toLocaleString(),
      p[0]! + 5,
      h - 12,
    );
  }
  for (
    let y = Math.ceil(extent[1]! / step) * step, n = 0;
    y <= extent[3]! && n < 100;
    y += step, n++
  ) {
    const p = map.getPixelFromCoordinate([extent[0]!, y]);
    if (!p) continue;
    context.beginPath();
    context.moveTo(0, p[1]!);
    context.lineTo(w, p[1]!);
    context.stroke();
    context.fillText(Number(y.toPrecision(10)).toLocaleString(), 8, p[1]! - 5);
  }
}
function showSelection(zoom = true) {
  highlight.clear();
  for (const detail of store.details) {
    const f = format.readFeature(detail) as Feature;
    f.set("contributor", true);
    highlight.addFeature(f);
  }
  if (store.selected)
    highlight.addFeature(
      format.readFeature({
        type: "Feature",
        geometry: store.selected.geometry,
        properties: {},
      }) as Feature,
    );
  if (store.inspection)
    highlight.addFeature(format.readFeature(store.inspection) as Feature);
  const extent = highlight.getExtent();
  if (zoom && map && extent && !highlight.isEmpty())
    map.getView().fit(extent, {
      padding: [70, 70, 70, 70],
      minResolution: 1e-8,
    });
  map?.renderSync();
}
onMounted(() => {
  build();
  observer = new ResizeObserver(() => {
    map?.updateSize();
    drawGrid();
  });
  if (target.value) observer.observe(target.value);
});
watch(
  () => [
    store.ws?.id,
    store.ws?.config.working_crs,
    store.ws?.config.view.basemap,
    store.ws?.config.layers.map((l) => `${l.id}:${l.selected}`).join(","),
  ],
  () => build(),
);
watch(
  () => store.ws?.config.layers,
  () => {
    for (const settings of store.ws?.config.layers ?? []) {
      layers.get(settings.id)?.setStyle(style(settings.color));
      layers.get(settings.id)?.setVisible(settings.visible);
    }
    void load();
  },
  { deep: true },
);
watch(() => store.fitRequest, fit);
watch(
  () => [store.selected, store.details, store.inspection],
  () => showSelection(),
  { deep: true },
);
onBeforeUnmount(() => {
  abort?.abort();
  observer?.disconnect();
  map?.dispose();
});
</script>
<template>
  <section class="viewer" aria-label="Geometry viewer">
    <div ref="target" class="map-target"></div>
    <canvas ref="canvas" class="coordinate-grid" aria-hidden="true"></canvas>
    <div class="map-heading">
      <span class="pill">{{
        store.ws?.config.view.basemap ? "OPENSTREETMAP" : "COORDINATE GRID"
      }}</span
      ><span v-if="loading" class="map-loading">Loading preview</span>
    </div>
    <button
      class="fit-button"
      @click="store.fit(null)"
      title="Fit all visible layers"
    >
      ⌖ <span>Fit layers</span>
    </button>
    <div v-if="!store.ws?.layers.length" class="map-welcome">
      <div class="welcome-symbol">⌘</div>
      <span class="eyebrow">A CLEARER VIEW OF YOUR DATA</span>
      <h1>Good geometry.<br />Sound topology.</h1>
      <p>
        Find gaps, crossings, and disconnected features.<br />Inspect every
        issue in its original context.
      </p>
      <button
        class="primary"
        :disabled="store.workspaceBusy"
        @click="store.openExamples"
      >
        Explore examples <span aria-hidden="true">↗</span>
      </button>
      <button
        class="text-button"
        :disabled="store.workspaceBusy"
        @click="store.loadExample()"
      >
        Synthetic topology tutorial
      </button>
      <small>Local analysis · Complete geometries · 38 rules</small>
    </div>
    <div v-if="truncated || loadError" class="preview-warning" role="status">
      {{
        loadError ||
        "Preview limited to 10,000 features or 200,000 coordinates per layer. Zoom closer to inspect more. Checks use every feature."
      }}
    </div>
    <div class="map-footer">
      <span>{{
        coordinates || "Pan to explore · Scroll to zoom · Click to inspect"
      }}</span
      ><span>{{
        store.ws?.config.view.basemap
          ? "Display coordinates in metres"
          : "Working coordinates"
      }}</span>
    </div>
    <div class="map-key">
      <span class="key-dot teal"></span> Source
      <span class="key-dot amber"></span> Contributor
      <span class="key-dot orange"></span> Violation
    </div>
  </section>
</template>
