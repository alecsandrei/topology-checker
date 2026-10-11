<script setup lang="ts">
import { useWorkspace } from "../store";
import type { LayerSettings } from "../types";
const store = useWorkspace();
async function change(
  layer: LayerSettings,
  field: "selected" | "visible" | "color" | "source_crs",
  value: boolean | string | null,
) {
  if (!store.draft) return;
  Object.assign(layer, { [field]: value });
  if (field === "selected" && !value)
    for (const c of store.draft.checks)
      if (c.layer === layer.id || c.other === layer.id) c.enabled = false;
  store.dirty = true;
  await store.apply();
  if (field === "selected") store.fit(layer.id);
}
function setting(id: string) {
  return store.draft!.layers.find((l) => l.id === id)!;
}
function provenance(dataset: string) {
  return store.ws?.datasets?.find((d) => d.id === dataset)?.provenance;
}
function crsLabel(def: string | null) {
  return def
    ?.match(/(?:ID|AUTHORITY)\["EPSG",\s*"?(\d+)"?\]/g)
    ?.at(-1)
    ?.match(/(\d+)/)?.[0]
    ? `EPSG:${
        def
          .match(/(?:ID|AUTHORITY)\["EPSG",\s*"?(\d+)"?\]/g)!
          .at(-1)!
          .match(/(\d+)/)![0]
      }`
    : def
      ? "Referenced"
      : "Unreferenced";
}
</script>
<template>
  <aside class="layer-panel">
    <div class="panel-title">
      <h2>Layers</h2>
      <span class="count-badge">{{ store.ws?.layers.length ?? 0 }}</span>
    </div>
    <p v-if="!store.ws?.layers.length" class="empty-note">
      Your datasets will appear here.<br />Import a file to start inspecting.
    </p>
    <p
      v-else-if="store.ws.layers.some((l) => !setting(l.id).selected)"
      class="layer-hint"
    >
      Select the dataset layers to include. Visibility only changes the preview.
    </p>
    <div class="layer-list">
      <article
        v-for="layer in store.ws?.layers"
        :key="layer.id"
        class="layer-card"
        :class="{ inactive: !setting(layer.id).selected }"
      >
        <div class="layer-card-heading">
          <input
            type="checkbox"
            :checked="setting(layer.id).selected"
            :aria-label="`Include ${layer.name}`"
            :disabled="store.busy"
            @change="
              change(
                setting(layer.id),
                'selected',
                ($event.target as HTMLInputElement).checked,
              )
            "
          /><span class="layer-name" :title="layer.name">{{ layer.name }}</span
          ><button
            class="icon-button"
            :disabled="!setting(layer.id).selected"
            :aria-label="`Fit ${layer.name}`"
            @click="store.fit(layer.id)"
          >
            ⌖
          </button>
        </div>
        <div class="layer-meta">
          {{ layer.feature_count.toLocaleString() }} features ·
          {{ layer.kinds.join(", ") || "empty" }}
        </div>
        <div class="layer-options">
          <label class="visibility"
            ><input
              type="checkbox"
              :checked="setting(layer.id).visible"
              :disabled="store.busy"
              @change="
                change(
                  setting(layer.id),
                  'visible',
                  ($event.target as HTMLInputElement).checked,
                )
              "
            />
            Show</label
          ><span>{{
            crsLabel(setting(layer.id).source_crs || layer.source_crs)
          }}</span
          ><input
            class="color-input"
            type="color"
            :value="setting(layer.id).color"
            :disabled="store.busy"
            :aria-label="`Color for ${layer.name}`"
            @input="
              change(
                setting(layer.id),
                'color',
                ($event.target as HTMLInputElement).value,
              )
            "
          />
        </div>
        <details
          v-if="provenance(layer.dataset)"
          class="source-details attribution"
        >
          <summary>Source & license</summary>
          <p>
            {{ provenance(layer.dataset)!.provider }} ·
            <a
              :href="provenance(layer.dataset)!.license_url"
              target="_blank"
              rel="noopener noreferrer"
              >{{ provenance(layer.dataset)!.license }}</a
            >
          </p>
          <a
            v-for="url in provenance(layer.dataset)!.source_urls"
            :key="url"
            :href="url"
            target="_blank"
            rel="noopener noreferrer"
            >{{ url.split("/").at(-1) }}</a
          >
          <p>Revision: {{ provenance(layer.dataset)!.revision }}</p>
          <p>Downloaded: {{ provenance(layer.dataset)!.download_date }}</p>
          <p>SHA-256: {{ provenance(layer.dataset)!.sha256 }}</p>
          <p v-if="provenance(layer.dataset)!.grouping">
            {{ provenance(layer.dataset)!.grouping }}
          </p>
          <p
            v-for="input in provenance(layer.dataset)!.inputs"
            :key="input.filename"
          >
            {{ input.filename }}: {{ input.sha256 }}
          </p>
        </details>
        <details class="source-details">
          <summary>Source CRS & warnings</summary>
          <label
            >Assign source CRS<input
              :value="setting(layer.id).source_crs ?? ''"
              placeholder="e.g. EPSG:28992"
              :disabled="store.busy"
              @change="
                change(
                  setting(layer.id),
                  'source_crs',
                  ($event.target as HTMLInputElement).value || null,
                )
              "
          /></label>
          <p
            v-for="warning in layer.warnings"
            :key="warning"
            class="warning-text"
          >
            {{ warning }}
          </p>
        </details>
      </article>
    </div>
    <div v-if="store.selected || store.inspection" class="inspection-panel">
      <div class="panel-title">
        <h2>{{ store.selected ? "Violation details" : "Feature details" }}</h2>
        <button
          class="icon-button"
          aria-label="Close inspection"
          @click="store.clearInspection"
        >
          ×
        </button>
      </div>
      <template v-if="store.selected"
        ><span class="category">{{ store.selected.category }}</span>
        <p class="inspection-rule">{{ store.selected.rule }}</p>
        <p v-if="!store.details.length" class="empty-note">
          This issue describes the layer as a whole.
        </p></template
      >
      <article
        v-for="feature in store.inspection ? [store.inspection] : store.details"
        :key="`${feature.layer}:${feature.id}`"
        class="feature-detail"
      >
        <h3>FID {{ feature.id }}</h3>
        <span class="feature-layer">{{
          store.ws?.layers.find((l) => l.id === feature.layer)?.name
        }}</span>
        <dl>
          <template v-for="(value, key) in feature.properties" :key="key"
            ><dt>{{ key }}</dt>
            <dd>
              {{
                typeof value === "object"
                  ? JSON.stringify(value)
                  : (value ?? "null")
              }}
            </dd></template
          >
        </dl>
      </article>
      <p
        v-if="store.selected?.origin?.part !== null && store.selected?.origin"
        class="part-reference"
      >
        Origin part {{ store.selected.origin.part
        }}<span
          v-if="
            store.selected.destination?.part !== null &&
            store.selected.destination
          "
        >
          · Destination part {{ store.selected.destination.part }}</span
        >
      </p>
    </div>
    <div class="local-note">
      <span class="status-dot"></span>
      <div>
        <strong>Stays on this computer</strong>
        <p>Workspaces are temporary.<br />Save a project to keep your setup.</p>
      </div>
    </div>
  </aside>
</template>
