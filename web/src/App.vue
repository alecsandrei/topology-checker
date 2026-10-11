<script setup lang="ts">
import { onMounted, ref } from "vue";
import {
  DropdownMenuRoot,
  DropdownMenuTrigger,
  DropdownMenuPortal,
  DropdownMenuContent,
  DropdownMenuItem,
} from "reka-ui";
import { useWorkspace } from "./store";
import ExamplesGallery from "./components/ExamplesGallery.vue";
import LayerPanel from "./components/LayerPanel.vue";
import MapViewer from "./components/MapViewer.vue";
import RulePanel from "./components/RulePanel.vue";
import ResultsPanel from "./components/ResultsPanel.vue";
const store = useWorkspace();
const uploadInput = ref<HTMLInputElement>();
const projectInput = ref<HTMLInputElement>();
const dragging = ref(false);
async function upload(event: Event) {
  const input = event.target as HTMLInputElement;
  for (const file of Array.from(input.files ?? [])) await store.upload(file);
  input.value = "";
}
async function openProject(event: Event) {
  const input = event.target as HTMLInputElement;
  if (input.files?.[0]) await store.replace("/projects/open", input.files[0]);
  input.value = "";
}
async function drop(event: DragEvent) {
  dragging.value = false;
  for (const file of Array.from(event.dataTransfer?.files ?? [])) {
    if (file.name.endsWith(".topology-project.zip"))
      await store.replace("/projects/open", file);
    else await store.upload(file);
  }
}
async function basemap() {
  if (!store.draft) return;
  store.draft.view.basemap = !store.ws?.config.view.basemap;
  store.draft.view.center = null;
  store.draft.view.resolution = null;
  await store.apply();
}
onMounted(() => store.initialize());
</script>
<template>
  <div
    class="app-shell"
    @dragover.prevent="dragging = true"
    @dragleave.self="dragging = false"
    @drop.prevent="drop"
  >
    <header class="app-header">
      <a class="brand" href="/" aria-label="Topology checker home"
        ><svg viewBox="0 0 32 32" aria-hidden="true">
          <path d="M7 8 24 6 26 24 10 26 7 8M7 8l19 16M24 6 10 26" />
          <circle cx="7" cy="8" r="3" />
          <circle cx="24" cy="6" r="3" />
          <circle cx="26" cy="24" r="3" />
          <circle cx="10" cy="26" r="3" /></svg
        ><span>topology<span class="brand-light">checker</span></span
        ><span class="local-badge">LOCAL</span></a
      >
      <nav aria-label="Project actions">
        <button
          class="text-button"
          :disabled="store.workspaceBusy"
          @click="store.openExamples"
        >
          Examples
        </button>
        <button
          class="text-button"
          :disabled="store.busy"
          @click="projectInput?.click()"
        >
          Open project</button
        ><button
          class="secondary"
          :disabled="store.busy || !store.ws?.layers.length"
          @click="store.save"
        >
          Save project</button
        ><button
          class="primary"
          :disabled="store.busy || !store.ws"
          @click="uploadInput?.click()"
        >
          <span>＋</span> Import dataset
        </button>
      </nav>
      <input
        ref="uploadInput"
        type="file"
        multiple
        :accept="store.importAccept"
        hidden
        @change="upload"
      /><input
        ref="projectInput"
        type="file"
        accept=".zip"
        hidden
        @change="openProject"
      />
    </header>
    <div class="workspace-bar">
      <div>
        <span
          class="status-dot"
          :class="{
            working: store.busy || store.ws?.job?.status === 'running',
          }"
        ></span
        ><strong>{{
          store.ws?.layers.length ? "Inspection workspace" : "New workspace"
        }}</strong
        ><span class="workspace-count"
          >{{ store.ws?.layers.length ?? 0 }} layers</span
        ><span v-if="store.busy" class="activity" role="status">Working…</span>
      </div>
      <div class="viewer-actions">
        <button
          class="view-toggle"
          :class="{ active: !store.ws?.config.view.basemap }"
          :disabled="store.busy || !store.ws?.config.working_crs"
          @click="basemap"
        >
          {{
            store.ws?.config.view.basemap ? "Switch to grid" : "Add basemap"
          }}</button
        ><DropdownMenuRoot
          ><DropdownMenuTrigger
            class="secondary export-trigger"
            :disabled="store.busy || !store.ws?.result"
            >Export report <span>⌄</span></DropdownMenuTrigger
          ><DropdownMenuPortal
            ><DropdownMenuContent class="dropdown" align="end" :side-offset="8"
              ><DropdownMenuItem
                class="dropdown-item"
                @select="store.exportReport('json')"
                >Complete JSON report</DropdownMenuItem
              ><DropdownMenuItem
                v-for="format in store.exportFormats"
                :key="format.extension"
                class="dropdown-item"
                :disabled="!store.ws?.config.working_crs"
                @select="store.exportReport(format.extension)"
                >{{ format.label }}</DropdownMenuItem
              ></DropdownMenuContent
            ></DropdownMenuPortal
          ></DropdownMenuRoot
        >
      </div>
    </div>
    <div v-if="store.error" class="error-banner" role="alert">
      <span>{{ store.error }}</span
      ><button aria-label="Dismiss error" @click="store.error = ''">×</button>
    </div>
    <main class="workspace-layout">
      <LayerPanel />
      <div class="center-panel"><MapViewer /><ResultsPanel /></div>
      <RulePanel />
    </main>
    <ExamplesGallery v-if="store.galleryOpen" />
    <div v-if="dragging" class="drop-overlay">
      <div>
        <span>＋</span>
        <h2>Drop your dataset here</h2>
        <p>
          GeoJSON, GeoPackage, FlatGeobuf, zipped Shapefile, or a saved project
        </p>
      </div>
    </div>
  </div>
</template>
