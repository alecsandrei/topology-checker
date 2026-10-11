import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { request } from "../src/api";
import { useWorkspace } from "../src/store";
import type { Page, Summary, Violation } from "../src/types";
import { fixture } from "./fixtures";

vi.mock("../src/api", async (original) => ({
  ...(await original<typeof import("../src/api")>()),
  request: vi.fn(),
}));
const api = vi.mocked(request);
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => (resolve = complete));
  return { promise, resolve };
}
function summary(): Summary {
  return {
    revision: 1,
    config: fixture().config,
    valid: false,
    total: 1,
    rules: [],
    warnings: [],
  };
}
function row(): Violation {
  return {
    id: 0,
    rule: "line/must-not-have-dangles",
    category: "dangle",
    geometry: { type: "Point", coordinates: [5, 52] },
    origin: { layer: "lines", name: "Streets", fid: "-12", part: null },
    destination: null,
  };
}
beforeEach(() => {
  setActivePinia(createPinia());
  api.mockReset();
  sessionStorage.clear();
  const store = useWorkspace();
  store.ws = fixture();
  store.draft = fixture().config;
});
afterEach(() => {
  vi.useRealTimers();
});

describe("workspace response ordering", () => {
  it("discards pages requested before configuration invalidated the results", async () => {
    const store = useWorkspace();
    const pending = deferred<Page>();
    api.mockReturnValueOnce(pending.promise);
    const page = store.fetchPage(0);
    const updated = fixture();
    updated.revision = 2;
    updated.config.tolerance = 0.1;
    api.mockResolvedValueOnce(updated);
    await store.apply(updated.config);
    pending.resolve({ page: 0, page_size: 100, total: 1, violations: [row()] });
    await page;
    expect(store.page).toBeNull();
    expect(store.ws!.revision).toBe(2);
  });

  it("keeps the latest requested page when requests complete out of order", async () => {
    const first = deferred<Page>();
    const second = deferred<Page>();
    api.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const store = useWorkspace();
    const a = store.fetchPage(0);
    const b = store.fetchPage(1);
    second.resolve({ page: 1, page_size: 100, total: 150, violations: [] });
    await b;
    first.resolve({ page: 0, page_size: 100, total: 150, violations: [] });
    await a;
    expect(store.page!.page).toBe(1);
  });

  it("rejects a completed poll from the previous configuration", async () => {
    vi.useFakeTimers();
    const store = useWorkspace();
    const workspace = fixture();
    workspace.job = {
      id: "job",
      revision: 1,
      status: "running",
      message: null,
      config: workspace.config,
    };
    api.mockResolvedValueOnce([]).mockResolvedValueOnce(workspace);
    await store.initialize();
    const pending = deferred<unknown>();
    api.mockReturnValueOnce(pending.promise);
    vi.advanceTimersByTime(400);
    const updated = fixture();
    updated.revision = 2;
    updated.config.tolerance = 0.1;
    api.mockResolvedValueOnce(updated);
    await store.apply(updated.config);
    pending.resolve({
      job: { ...workspace.job, status: "completed" },
      result: summary(),
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(store.ws!.result).toBeNull();
    expect(store.page).toBeNull();
    expect(api).toHaveBeenCalledTimes(4);
  });

  it("keeps controls busy until all overlapping operations finish", async () => {
    const store = useWorkspace();
    const first = deferred<void>();
    const second = deferred<void>();
    const a = store.perform(() => first.promise);
    const b = store.perform(() => second.promise);
    first.resolve();
    await a;
    expect(store.busy).toBe(true);
    second.resolve();
    await b;
    expect(store.busy).toBe(false);
  });

  it("does not reopen an inspection that was closed before its request completed", async () => {
    const store = useWorkspace();
    const pending = deferred<unknown>();
    api.mockReturnValueOnce(pending.promise);
    const inspection = store.inspect("lines", "-12");
    store.clearInspection();
    pending.resolve({
      type: "Feature",
      layer: "lines",
      id: "-12",
      geometry: row().geometry,
      properties: {},
    });
    await inspection;
    expect(store.inspection).toBeNull();
    expect(store.selected).toBeNull();
  });
});

describe("configuration and display", () => {
  it("applies pending rule changes before importing another dataset", async () => {
    const store = useWorkspace();
    store.draft!.checks = [
      {
        rule: "line/must-not-have-dangles",
        layer: "lines",
        other: null,
        enabled: true,
      },
    ];
    store.dirty = true;
    const updated = fixture();
    updated.config = JSON.parse(JSON.stringify(store.draft));
    api.mockResolvedValueOnce(updated).mockResolvedValueOnce(updated);
    await store.upload(new File(["{}"], "new.geojson"));
    expect(api.mock.calls.map(([path]) => path)).toEqual([
      "/workspaces/workspace/config",
      "/workspaces/workspace/uploads",
    ]);
    expect(JSON.parse(String(api.mock.calls[0]![1]!.body)).checks).toHaveLength(
      1,
    );
    expect(store.draft!.checks).toHaveLength(1);
  });

  it("does not upload when pending configuration is invalid", async () => {
    const store = useWorkspace();
    store.dirty = true;
    api.mockRejectedValueOnce(new Error("invalid rule input"));
    await store.upload(new File(["{}"], "new.geojson"));
    expect(api).toHaveBeenCalledTimes(1);
    expect(store.error).toBe("invalid rule input");
    expect(store.dirty).toBe(true);
  });

  it("reloads selected geometry and contributors in the new display coordinates", async () => {
    const store = useWorkspace();
    store.selected = row();
    const projected = row();
    projected.geometry = { type: "Point", coordinates: [556597, 6800125] };
    const updated = fixture();
    updated.config.view.basemap = true;
    api
      .mockResolvedValueOnce(updated)
      .mockResolvedValueOnce(projected)
      .mockResolvedValueOnce({
        type: "Feature",
        layer: "lines",
        id: "-12",
        geometry: projected.geometry,
        properties: {},
      });
    await store.apply(updated.config);
    await vi.waitFor(() =>
      expect(store.selected?.geometry).toEqual(projected.geometry),
    );
    expect(api.mock.calls.slice(1).map(([path]) => path)).toEqual([
      "/workspaces/workspace/violations/0?display=true",
      "/workspaces/workspace/layers/lines/features/-12?display=true",
    ]);
    expect(store.details[0]!.geometry).toEqual(projected.geometry);
  });
});

describe("example loading", () => {
  it("keeps the current workspace, results and gallery when loading fails", async () => {
    const store = useWorkspace();
    store.ws!.result = summary();
    store.galleryOpen = true;
    api.mockRejectedValueOnce(new Error("Example import failed"));
    await store.loadExample("world-cities");
    expect(store.ws!.id).toBe("workspace");
    expect(store.ws!.result).toEqual(summary());
    expect(store.galleryOpen).toBe(true);
    expect(store.error).toBe("Example import failed");
    expect(store.busy).toBe(false);
  });
  it("ignores selections while an operation is active", async () => {
    const store = useWorkspace();
    store.busy = true;
    await store.loadExample("major-rivers");
    expect(api).not.toHaveBeenCalled();
  });
  it("opens the gallery with catalog metadata and allows retry after failure", async () => {
    const store = useWorkspace();
    api.mockRejectedValueOnce(new Error("Catalog unavailable"));
    await store.openExamples();
    expect(store.galleryOpen).toBe(true);
    expect(store.error).toBe("Catalog unavailable");
    api.mockResolvedValueOnce([]);
    await store.openExamples();
    expect(store.error).toBe("");
    expect(api).toHaveBeenLastCalledWith("/examples");
  });
});
