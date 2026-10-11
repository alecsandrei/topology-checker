import { createHash } from "node:crypto";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

const root = fileURLToPath(
  new URL("../crates/server/examples/", import.meta.url),
);
const revision = "ca96624a56bd078437bca8184e78163e5039ad19";
const license_url = "https://www.naturalearthdata.com/about/terms-of-use/";
const hashes = {
  ne_110m_populated_places:
    "a86028b083182b68c7620fc6e1a8a47ee547cb9cd2fb62ccbb78bea786440899",
  ne_110m_rivers_lake_centerlines:
    "55aa4497405afc07cdc931b7fbe062c4d6693ba2a550c0d24899953f5d507c8d",
  ne_50m_rivers_lake_centerlines:
    "f286e0ce978fde999ca2d7a78c764be08542e19b63cded52b05c12d5173ccc51",
  ne_110m_admin_0_countries:
    "6866c877d39cba9c357620878839b336d569f8c662d3cfab4cb1dbe2d39c977f",
  ne_110m_lakes:
    "eb02ecc86c82004fccbf979058bfabbbd6c2d07968c7844d38eb1c9152d2ffc9",
};
const offline = process.argv.includes("--check");
const source = {};
const metadata = {};
let previous = [];
try {
  previous = JSON.parse(await readFile(`${root}catalog.json`, "utf8"));
} catch (e) {
  if (e.code !== "ENOENT") throw e;
}
const priorDates = new Map(
  previous.flatMap((e) =>
    e.datasets.map((d) => [d.filename, d.provenance.download_date]),
  ),
);
const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
await mkdir(`${root}sources`, { recursive: true });
await mkdir(`${root}generated`, { recursive: true });
for (const [name, expected] of Object.entries(hashes)) {
  const filename = `${name}.geojson`;
  const url = `https://raw.githubusercontent.com/nvkelso/natural-earth-vector/${revision}/geojson/${filename}`;
  let bytes;
  if (offline) bytes = await readFile(`${root}sources/${filename}`);
  else {
    const response = await fetch(url);
    if (!response.ok)
      throw Error(`Download failed for ${filename}: ${response.status}`);
    bytes = Buffer.from(await response.arrayBuffer());
  }
  assert.equal(sha(bytes), expected, `SHA-256 mismatch for ${filename}`);
  if (!offline) await writeFile(`${root}sources/${filename}`, bytes);
  const data = JSON.parse(bytes);
  source[name] = data.features;
  metadata[name] = {
    filename,
    asset: `sources/${filename}`,
    feature_count: data.features.length,
    imported_feature_count: data.features.filter(
      (f) =>
        f.geometry &&
        (f.geometry.coordinates?.length || f.geometry.geometries?.length),
    ).length,
    geometry_types: [
      ...new Set(data.features.map((f) => f.geometry.type)),
    ].sort(),
    provenance: {
      provider: "Natural Earth",
      source_urls: [url],
      revision,
      sha256: expected,
      download_date:
        priorDates.get(filename) ?? new Date().toISOString().slice(0, 10),
      license: "Public domain",
      license_url,
      grouping: null,
    },
  };
}
const citiesName = "ne_110m_populated_places";
const countriesName = "ne_110m_admin_0_countries";
const cities = source[citiesName];
const countries = source[countriesName];
assert.equal(cities.length, 243);
assert.equal(source.ne_110m_rivers_lake_centerlines.length, 13);
assert.equal(source.ne_50m_rivers_lake_centerlines.length, 462);
assert.equal(
  countries.filter((f) => f.geometry.type === "Polygon").length,
  148,
);
assert.equal(
  countries.filter((f) => f.geometry.type === "MultiPolygon").length,
  29,
);
assert.equal(source.ne_110m_lakes.length, 24);
// Source identifiers are Natural Earth's NE_ID when present, otherwise a zero-based source ordinal.
const member = (name, feature, ordinal) => ({
  source_dataset: `${name}.geojson`,
  source_identifier: feature.id ?? feature.properties.NE_ID ?? ordinal,
  source_ordinal: ordinal,
  attributes: feature.properties,
});
const groups = new Map();
cities.forEach((f, i) => {
  const code = f.properties.ADM0_A3;
  if (!groups.has(code)) groups.set(code, []);
  groups.get(code).push({ feature: f, ordinal: i });
});
const multipoints = [...groups.keys()]
  .sort()
  .map((code, i) => ({
    type: "Feature",
    id: i + 1,
    properties: {
      ADM0_A3: code,
      members: groups
        .get(code)
        .map((m) => member(citiesName, m.feature, m.ordinal)),
    },
    geometry: {
      type: "MultiPoint",
      coordinates: groups.get(code).map((m) => m.feature.geometry.coordinates),
    },
  }));
const collections = countries.map((country, i) => {
  const matches = groups.get(country.properties.ADM0_A3) ?? [];
  return {
    type: "Feature",
    id: i + 1,
    properties: {
      ADM0_A3: country.properties.ADM0_A3,
      name: country.properties.NAME,
      members: [
        member(countriesName, country, i),
        ...matches.map((m) => member(citiesName, m.feature, m.ordinal)),
      ],
    },
    geometry: {
      type: "GeometryCollection",
      geometries: [country.geometry, ...matches.map((m) => m.feature.geometry)],
    },
  };
});
async function derived(name, features, inputs, grouping) {
  const filename = `${name}.geojson`;
  const bytes = Buffer.from(
    JSON.stringify({ type: "FeatureCollection", features }) + "\n",
  );
  if (offline)
    assert.deepEqual(
      await readFile(`${root}generated/${filename}`),
      bytes,
      `Regenerate ${filename}`,
    );
  else await writeFile(`${root}generated/${filename}`, bytes);
  return {
    filename,
    asset: `generated/${filename}`,
    feature_count: features.length,
    imported_feature_count: features.length,
    geometry_types: [features[0].geometry.type],
    provenance: {
      ...metadata[inputs[0]].provenance,
      source_urls: inputs.flatMap((n) => metadata[n].provenance.source_urls),
      sha256: sha(bytes),
      grouping,
      inputs: inputs.map((n) => ({
        filename: metadata[n].filename,
        sha256: metadata[n].provenance.sha256,
      })),
    },
  };
}
const grouped = await derived(
  "cities_by_country",
  multipoints,
  [citiesName],
  "Group cities by ADM0_A3 in sorted country-code order. Coordinates and original member attributes are preserved without clipping, simplification, merging or movement.",
);
const atlas = await derived(
  "country_city_atlas",
  collections,
  [countriesName, citiesName],
  "One GeometryCollection per country in source order, containing the original country geometry followed by cities whose ADM0_A3 matches. Matching uses country codes, not spatial containment. All coordinates and member attributes are preserved.",
);
const multipart = "geometry/must-not-be-multipart";
const make = (id, title, description, datasets, checks, note = "") => ({
  id,
  title,
  description,
  datasets: datasets.map((d, i) => ({
    ...d,
    visible: !(
      (id === "world-cities" && i === 1) ||
      (id === "major-rivers" && i === 0)
    ),
    checks: checks[i],
  })),
  note,
});
const catalog = [
  make(
    "world-cities",
    "World cities",
    "243 populated places and country groups made from their original points.",
    [metadata[citiesName], grouped],
    [["point/must-not-overlap"], [multipart]],
  ),
  make(
    "major-rivers",
    "Major rivers",
    "River centerlines at 110m and 50m scales, checked separately.",
    [
      metadata.ne_110m_rivers_lake_centerlines,
      metadata.ne_50m_rivers_lake_centerlines,
    ],
    Array.from({ length: 2 }, () => [
      "line/must-not-have-dangles",
      "line/must-not-self-intersect",
    ]),
    "The 50m source has 462 records; one empty Loire geometry is skipped on import, leaving 461 usable features. River endpoints are useful inspection findings. Their suitability depends on the selected rule and network purpose.",
  ),
  make(
    "countries-islands",
    "Countries and islands",
    "177 countries, including 148 Polygons and 29 MultiPolygons.",
    [metadata[countriesName]],
    [["polygon/must-not-overlap", multipart]],
    "Multipart countries represent islands and separated territories. Their suitability depends on the selected rule.",
  ),
  make(
    "lakes-reservoirs",
    "Lakes and reservoirs",
    "24 lake and reservoir polygons at 110m scale.",
    [metadata.ne_110m_lakes],
    [["polygon/must-not-overlap"]],
  ),
  make(
    "country-city-atlas",
    "Country and city atlas",
    "177 country collections with original country geometry and cities matched by country code.",
    [atlas],
    [[multipart]],
    "Collections preserve country and city members. Multipart findings are reported against the parent feature.",
  ),
];
const output = JSON.stringify(catalog, null, 2) + "\n";
if (offline)
  assert.equal(
    await readFile(`${root}catalog.json`, "utf8"),
    output,
    "Catalog differs, regenerate examples",
  );
else await writeFile(`${root}catalog.json`, output);
console.log(
  `Verified five sources, two derived datasets and ${catalog.length} examples.`,
);
