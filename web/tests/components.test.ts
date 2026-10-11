import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import { compatible, useWorkspace } from "../src/store";
import RulePanel from "../src/components/RulePanel.vue";
import ResultsPanel from "../src/components/ResultsPanel.vue";
import LayerPanel from "../src/components/LayerPanel.vue";
import { fixture } from "./fixtures";
import type { Example, Violation } from "../src/types";
beforeEach(() => {
  setActivePinia(createPinia());
  const store = useWorkspace();
  store.ws = fixture();
  store.draft = fixture().config;
  store.rules = [
    {
      name: "line/must-not-have-dangles",
      description: "Find disconnected ends.",
      primary: "line",
      secondary: null,
    },
    {
      name: "point/must-be-covered-by-lines",
      description: "Cover stops.",
      primary: "point",
      secondary: "line",
    },
    {
      name: "polygon/must-not-overlap",
      description: "Find overlaps.",
      primary: "polygon",
      secondary: null,
    },
  ];
});
describe("rule configuration", () => {
  it("filters incompatible rules and creates explicit input bindings", async () => {
    const wrapper = mount(RulePanel);
    expect(wrapper.find('[aria-label="Rule to add"]').text()).not.toContain(
      "polygon/must-not-overlap",
    );
    await wrapper
      .find('[aria-label="Rule to add"]')
      .setValue("point/must-be-covered-by-lines");
    await wrapper
      .findAll("button")
      .find((b) => b.text().includes("Add rule"))!
      .trigger("click");
    expect(useWorkspace().draft!.checks).toEqual([
      {
        rule: "point/must-be-covered-by-lines",
        layer: "points",
        other: "lines",
        enabled: true,
      },
    ]);
    expect(useWorkspace().dirty).toBe(true);
  });
  it("does not treat a mixed family as compatible with a unary family", () => {
    expect(compatible(["point", "line"], "line")).toBe(false);
    expect(compatible(["point", "line"], "any")).toBe(true);
  });
});
describe("inspection results", () => {
  it("shows signed large IDs intact and selects one aggregated violation", async () => {
    const store = useWorkspace();
    store.ws!.result = {
      revision: 1,
      config: store.ws!.config,
      valid: false,
      total: 1,
      warnings: [],
      rules: [],
    };
    const row: Violation = {
      id: 0,
      rule: "line/must-not-have-dangles",
      category: "dangle",
      geometry: {
        type: "MultiPoint" as const,
        coordinates: [
          [0, 0],
          [1, 1],
        ],
      },
      origin: {
        layer: "lines",
        name: "Streets",
        fid: "9007199254740993",
        part: null,
        dataset: "d",
        selector: "lines",
        attributes: null,
      },
      destination: null,
    };
    store.page = { page: 0, page_size: 100, total: 1, violations: [row] };
    const select = vi.spyOn(store, "select").mockResolvedValue();
    const wrapper = mount(ResultsPanel);
    expect(wrapper.text()).toContain("9007199254740993");
    expect(wrapper.findAll("tbody tr")).toHaveLength(1);
    await wrapper.find("tbody tr").trigger("keydown.enter");
    expect(select).toHaveBeenCalledWith(row);
  });
  it("shows clean reports and their warnings", () => {
    const store = useWorkspace();
    store.ws!.result = {
      revision: 1,
      config: store.ws!.config,
      valid: true,
      total: 0,
      warnings: ["Missing CRS"],
      rules: [],
    };
    const wrapper = mount(ResultsPanel);
    expect(wrapper.text()).toContain("All configured checks passed");
    expect(wrapper.text()).toContain("Missing CRS");
  });
});
describe("feature inspection", () => {
  it("shows original attributes, null values, and signed identities", () => {
    const store = useWorkspace();
    store.inspection = {
      type: "Feature",
      layer: "lines",
      id: "-12",
      geometry: { type: "Point", coordinates: [0, 0] },
      properties: { name: "Canal Street", nullable: null },
    };
    const wrapper = mount(LayerPanel);
    expect(wrapper.text()).toContain("FID -12");
    expect(wrapper.text()).toContain("Canal Street");
    expect(wrapper.text()).toContain("null");
  });
});

describe("examples gallery", () => {
  it("shows all catalog metadata and disables selection during workspace operations", async () => {
    const { default: ExamplesGallery } =
      await import("../src/components/ExamplesGallery.vue");
    const { default: catalog } =
      await import("../../crates/server/examples/catalog.json");
    const store = useWorkspace();
    store.examples = catalog as Example[];
    store.galleryOpen = true;
    const load = vi.spyOn(store, "loadExample").mockResolvedValue();
    const wrapper = mount(ExamplesGallery, {
      attachTo: document.body,
      global: { stubs: { DialogPortal: { template: "<div><slot /></div>" } } },
    });
    await wrapper.vm.$nextTick();
    expect(wrapper.findAll(".example-card")).toHaveLength(5);
    for (const kind of [
      "Point",
      "MultiPoint",
      "LineString",
      "MultiLineString",
      "Polygon",
      "MultiPolygon",
      "GeometryCollection",
    ])
      expect(wrapper.text()).toContain(kind);
    expect(wrapper.text()).toContain("243 features");
    expect(wrapper.text()).toContain("Public domain");
    expect(wrapper.find('a[href*="ca96624"]').exists()).toBe(true);
    await wrapper.find(".example-card button").trigger("click");
    expect(load).toHaveBeenCalledWith("world-cities");
    store.busy = true;
    await wrapper.vm.$nextTick();
    expect(
      wrapper
        .findAll(".example-card button")
        .every((b) => b.attributes("disabled") !== undefined),
    ).toBe(true);
    store.busy = false;
    store.ws!.job = {
      id: "job",
      status: "running",
      revision: 1,
      message: null,
      config: store.ws!.config,
    };
    await wrapper.vm.$nextTick();
    expect(
      wrapper.find(".example-card button").attributes("disabled"),
    ).toBeDefined();
    wrapper.unmount();
  });
});
