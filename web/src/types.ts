import type { Feature, Geometry } from "geojson";
export type Kind = "point" | "line" | "polygon" | "collection" | "any";
export interface Rule {
  name: string;
  description: string;
  primary: Kind;
  secondary: Kind | null;
}
export interface Check {
  rule: string;
  layer: string;
  other: string | null;
  enabled: boolean;
}
export interface LayerSettings {
  id: string;
  selected: boolean;
  visible: boolean;
  color: string;
  source_crs: string | null;
}
export interface Config {
  working_crs: string | null;
  tolerance: number;
  layers: LayerSettings[];
  checks: Check[];
  view: {
    basemap: boolean;
    center: [number, number] | null;
    resolution: number | null;
  };
}
export interface Layer {
  id: string;
  dataset: string;
  selector: string;
  name: string;
  feature_count: number;
  kinds: Kind[];
  source_crs: string | null;
  warnings: string[];
  extent: [number, number, number, number] | null;
  display_extent: [number, number, number, number] | null;
}
export interface Ref {
  layer: string;
  name: string;
  fid: string;
  part: number | null;
}
export interface Violation {
  id: number;
  rule: string;
  category: string;
  geometry: Geometry;
  origin: Ref | null;
  destination: Ref | null;
}
export interface Summary {
  revision: number;
  config: Config;
  valid: boolean;
  total: number;
  warnings: string[];
  rules: {
    rule: string;
    layer: string;
    other: string | null;
    counts: Record<string, number>;
    total: number;
  }[];
}
export interface Job {
  id: string;
  revision: number;
  status: "running" | "completed" | "failed" | "stale";
  message: string | null;
  config: Config;
}
export interface Provenance {
  provider: string;
  source_urls: string[];
  revision: string;
  sha256: string;
  download_date: string;
  license: string;
  license_url: string;
  grouping: string | null;
  inputs?: { filename: string; sha256: string }[];
}
export interface Dataset {
  id: string;
  filename: string;
  provenance: Provenance | null;
}
export interface Example {
  id: string;
  title: string;
  description: string;
  note: string;
  datasets: {
    filename: string;
    feature_count: number;
    imported_feature_count: number;
    geometry_types: string[];
    provenance: Provenance;
    checks: string[];
    visible: boolean;
  }[];
}
export interface Workspace {
  datasets?: Dataset[];
  id: string;
  revision: number;
  config: Config;
  working_crs_info: {
    label: string;
    units: string;
    default_tolerance: number;
  } | null;
  layers: Layer[];
  job: Job | null;
  result: Summary | null;
}
export interface Page {
  page: number;
  page_size: number;
  total: number;
  violations: Violation[];
}
export type MapFeature = Feature<Geometry> & { layer: string; id: string };
export interface Viewport {
  type: "FeatureCollection";
  features: MapFeature[];
  incomplete: boolean;
  coordinates: number;
}
