// Smoke-test the RustGIS plugin module through its real ABI (rustgis_alloc / rustgis_call), as the host would.
// Usage: node scripts/rustgis_plugin_smoke.mjs PLUGIN.wasm [--scale N]
//   --scale N also times a tool run on an N x N grid of squares (default: skipped).
import fs from "node:fs";

const args = process.argv.slice(2);
const wasmPath = args[0];
const scale = args.includes("--scale") ? Number(args[args.indexOf("--scale") + 1]) : 0;
if (!wasmPath) {
  console.error("usage: node rustgis_plugin_smoke.mjs PLUGIN.wasm [--scale N]");
  process.exit(2);
}

const module = new WebAssembly.Module(fs.readFileSync(wasmPath));

function call(request) {
  // A fresh instance per call, as RustGIS does.
  const { exports: ex } = new WebAssembly.Instance(module, {
    rustgis: { call: () => -1, result: () => {}, log: () => {} },
  });
  const bytes = new TextEncoder().encode(JSON.stringify(request));
  const ptr = ex.rustgis_alloc(bytes.length);
  new Uint8Array(ex.memory.buffer, ptr, bytes.length).set(bytes);
  const packed = ex.rustgis_call(ptr, bytes.length);
  const outPtr = Number(packed >> 32n);
  const outLen = Number(packed & 0xffffffffn);
  return JSON.parse(new TextDecoder().decode(new Uint8Array(ex.memory.buffer, outPtr, outLen)));
}

const rect = (x0, y0, x1, y1) => [[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]];
const polys = (rects) => ({
  type: "FeatureCollection",
  features: rects.map((r, row) => ({
    type: "Feature",
    properties: { row },
    geometry: { type: "Polygon", coordinates: [rect(...r)] },
  })),
});
const tool = (id, params, layers) => ({ abi: 1, kind: "tool", id, params, layers, lang: "en" });

let failed = false;
const check = (name, ok, detail = "") => {
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${detail ? ` ${detail}` : ""}`);
  failed ||= !ok;
};

// Four rectangles around an empty square: one gap.
const ring = polys([[0, 0, 3, 1], [0, 2, 3, 3], [0, 1, 1, 2], [2, 1, 3, 2]]);
let r = call(tool("polygon-must-not-have-gaps", { layer: "Parcels", tolerance: 1e-9 }, { Parcels: ring }));
const kinds = (r.ok?.layers?.[0]?.geojson?.features ?? []).map((f) => f.properties.error_type);
check("gaps found", kinds.includes("gap"), JSON.stringify(kinds));

r = call(tool("polygon-must-not-have-gaps", { layer: "Parcels" }, {}));
check("missing layer is an error, not a trap", typeof r.error === "string", r.error);

r = call({ abi: 1, kind: "tool", id: "no-such-tool", params: {}, layers: {}, lang: "en" });
check("unknown tool is an error", typeof r.error === "string", r.error);

if (scale > 0) {
  const rects = [];
  for (let i = 0; i < scale; i++) for (let j = 0; j < scale; j++) rects.push([i, j, i + 1, j + 1]);
  for (const id of ["polygon-must-not-overlap", "polygon-must-not-have-gaps"]) {
    const t0 = performance.now();
    r = call(tool(id, { layer: "Grid", tolerance: 1e-9 }, { Grid: polys(rects) }));
    const ms = Math.round(performance.now() - t0);
    check(`${id} on ${rects.length} polygons`, r.ok !== undefined, `${ms} ms ${r.error ?? ""}`);
  }
}
process.exit(failed ? 1 : 0);
