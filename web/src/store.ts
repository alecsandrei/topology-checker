import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { download, jsonBody, request } from "./api";
import type {
  Config,
  Example,
  Job,
  MapFeature,
  Page,
  Rule,
  Summary,
  Violation,
  Workspace,
} from "./types";
export function compatible(kinds: string[], required: string): boolean {
  return required === "any" || kinds.every((k) => k === required);
}
export const useWorkspace = defineStore("workspace", () => {
  const ws = ref<Workspace | null>(null);
  const rules = ref<Rule[]>([]);
  const busy = ref(false);
  const examples = ref<Example[]>([]);
  const galleryOpen = ref(false);
  const workspaceBusy = computed(
    () => busy.value || ws.value?.job?.status === "running",
  );
  async function openExamples() {
    galleryOpen.value = true;
    if (!examples.value.length)
      await perform(async () => {
        examples.value = await request<Example[]>("/examples");
      });
  }
  async function loadExample(id?: string) {
    if (workspaceBusy.value) return;
    if (await replace(id ? `/examples/${id}` : "/example"))
      galleryOpen.value = false;
  }
  const error = ref("");
  const page = ref<Page | null>(null);
  const selected = ref<Violation | null>(null);
  const details = ref<MapFeature[]>([]);
  const inspection = ref<MapFeature | null>(null);
  const fitLayer = ref<string | null>(null);
  const fitRequest = ref(0);
  const draft = ref<Config | null>(null);
  const dirty = ref(false);
  let pollTimer: ReturnType<typeof setTimeout> | undefined;
  let selectionVersion = 0;
  let contextVersion = 0;
  let pageVersion = 0;
  let pollVersion = 0;
  let operations = 0;
  async function perform<T>(work: () => Promise<T>): Promise<T | undefined> {
    const context = contextVersion;
    operations++;
    busy.value = true;
    error.value = "";
    try {
      return await work();
    } catch (e) {
      if (context === contextVersion)
        error.value = e instanceof Error ? e.message : String(e);
    } finally {
      busy.value = --operations > 0;
    }
  }
  function adopt(workspace: Workspace) {
    const changed =
      ws.value?.id !== workspace.id || ws.value.revision !== workspace.revision;
    const displayChanged =
      !changed &&
      ws.value?.config.view.basemap !== workspace.config.view.basemap;
    const priorSelection = selected.value;
    const priorInspection = inspection.value;
    ws.value = workspace;
    draft.value = JSON.parse(JSON.stringify(workspace.config));
    dirty.value = false;
    if (changed) {
      contextVersion++;
      pageVersion++;
      clearTimeout(pollTimer);
      pollVersion++;
      page.value = null;
      if (workspace.job?.status === "running") schedulePoll(workspace.id);
    }
    if (changed || displayChanged) {
      selected.value = null;
      details.value = [];
      inspection.value = null;
      selectionVersion++;
    }
    if (displayChanged) {
      if (priorSelection) void select(priorSelection);
      else if (priorInspection)
        void inspect(priorInspection.layer, priorInspection.id);
    }
  }
  async function initialize() {
    await perform(async () => {
      rules.value = await request<Rule[]>("/rules");
      const saved = sessionStorage.getItem("topology-workspace");
      if (saved) {
        try {
          adopt(await request<Workspace>(`/workspaces/${saved}`));
        } catch {
          adopt(await request<Workspace>("/workspaces", { method: "POST" }));
        }
      } else {
        adopt(await request<Workspace>("/workspaces", { method: "POST" }));
      }
      sessionStorage.setItem("topology-workspace", ws.value!.id);
      if (ws.value!.result) await fetchPage(0);
      if (ws.value!.job?.status === "running") schedulePoll(ws.value!.id);
    });
  }
  async function replace(path: string, file?: File) {
    let succeeded = false;
    await perform(async () => {
      const body = file ? new FormData() : undefined;
      if (file) body!.append("file", file);
      const workspace = await request<Workspace>(path, {
        method: "POST",
        body,
      });
      const previous = ws.value?.id;
      clearTimeout(pollTimer);
      pollVersion++;
      adopt(workspace);
      sessionStorage.setItem("topology-workspace", workspace.id);
      if (previous && previous !== workspace.id)
        await fetch(`/api/workspaces/${previous}`, { method: "DELETE" }).catch(
          () => undefined,
        );
      fit(null);
      succeeded = true;
    });
    return succeeded;
  }
  async function upload(file: File) {
    if (file.size > 512 * 1024 * 1024) {
      error.value = "Upload exceeds 512 MiB";
      return;
    }
    if (dirty.value && !(await apply())) return;
    await replace(`/workspaces/${ws.value!.id}/uploads`, file);
  }
  async function apply(config = draft.value!): Promise<boolean> {
    let succeeded = false;
    await perform(async () => {
      adopt(
        await request<Workspace>(
          `/workspaces/${ws.value!.id}/config`,
          jsonBody(config),
        ),
      );
      succeeded = true;
    });
    return succeeded;
  }
  async function run() {
    if (dirty.value && !(await apply())) return;
    await perform(async () => {
      await request(`/workspaces/${ws.value!.id}/runs`, { method: "POST" });
      ws.value!.job = {
        id: "",
        revision: ws.value!.revision,
        status: "running",
        message: null,
        config: JSON.parse(JSON.stringify(ws.value!.config)),
      };
      ws.value!.result = null;
      contextVersion++;
      pageVersion++;
      page.value = null;
      selected.value = null;
      details.value = [];
      selectionVersion++;
      schedulePoll(ws.value!.id);
    });
  }
  function schedulePoll(id: string) {
    clearTimeout(pollTimer);
    const version = ++pollVersion;
    pollTimer = setTimeout(async () => {
      try {
        const response = await request<{
          job: Job | null;
          result: Summary | null;
        }>(`/workspaces/${id}/job`);
        if (ws.value?.id !== id || version !== pollVersion) return;
        ws.value.job = response.job;
        ws.value.result =
          response.result?.revision === ws.value.revision
            ? response.result
            : null;
        if (response.job?.status === "running") schedulePoll(id);
        else if (response.job?.status === "completed" && ws.value.result)
          await fetchPage(0);
        else if (response.job?.message) error.value = response.job.message;
      } catch (e) {
        if (ws.value?.id === id && version === pollVersion)
          error.value = String(e);
      }
    }, 400);
  }
  async function fetchPage(number: number) {
    const id = ws.value!.id;
    const context = contextVersion;
    const version = ++pageVersion;
    try {
      const result = await request<Page>(
        `/workspaces/${id}/results?page=${number}`,
      );
      if (context === contextVersion && version === pageVersion)
        page.value = result;
    } catch (e) {
      if (context === contextVersion && version === pageVersion) throw e;
    }
  }
  async function select(row: Violation) {
    const version = ++selectionVersion;
    await perform(async () => {
      const display = ws.value!.config.view.basemap;
      const value = await request<Violation>(
        `/workspaces/${ws.value!.id}/violations/${row.id}?display=${display}`,
      );
      if (version !== selectionVersion) return;
      const features = await Promise.all(
        [value.origin, value.destination]
          .filter((r) => r !== null)
          .map((r) =>
            request<MapFeature>(
              `/workspaces/${ws.value!.id}/layers/${r.layer}/features/${r.fid}?display=${display}`,
            ),
          ),
      );
      if (version !== selectionVersion) return;
      selected.value = value;
      details.value = features;
      inspection.value = null;
    });
  }
  async function inspect(layer: string, fid: string) {
    const version = ++selectionVersion;
    await perform(async () => {
      const feature = await request<MapFeature>(
        `/workspaces/${ws.value!.id}/layers/${layer}/features/${fid}?display=${ws.value!.config.view.basemap}`,
      );
      if (version !== selectionVersion) return;
      inspection.value = feature;
      selected.value = null;
      details.value = [];
    });
  }
  function clearInspection() {
    selectionVersion++;
    selected.value = null;
    inspection.value = null;
    details.value = [];
  }
  function fit(id: string | null) {
    fitLayer.value = id;
    fitRequest.value++;
  }
  async function exportReport(format: string) {
    await perform(() =>
      download(
        `/workspaces/${ws.value!.id}/export/${format}`,
        `topology-report.${format}`,
      ),
    );
  }
  async function save() {
    if (!(await apply())) return;
    await perform(() =>
      download(
        `/workspaces/${ws.value!.id}/project`,
        "workspace.topology-project.zip",
      ),
    );
  }
  return {
    ws,
    examples,
    galleryOpen,
    workspaceBusy,
    openExamples,
    loadExample,
    rules,
    busy,
    error,
    page,
    selected,
    details,
    inspection,
    fitLayer,
    fitRequest,
    draft,
    dirty,
    perform,
    initialize,
    replace,
    upload,
    apply,
    run,
    fetchPage,
    select,
    inspect,
    clearInspection,
    fit,
    exportReport,
    save,
  };
});
