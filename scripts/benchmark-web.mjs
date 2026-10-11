import assert from 'node:assert/strict';
import { setTimeout as delay } from 'node:timers/promises';

const base = 'http://127.0.0.1:8080/api';
const count = Number(process.argv[2] ?? 200000);
assert(Number.isSafeInteger(count) && count > 0);
async function request(path, options) {
  const response = await fetch(base + path, options);
  const json = await response.json();
  assert(response.ok, JSON.stringify(json));
  return json;
}
async function health(metrics) {
  const start = performance.now();
  await request('/health');
  metrics.requests++;
  metrics.maximum_ms = Math.max(metrics.maximum_ms, performance.now() - start);
}
for (const family of ['point', 'line']) {
  const workspace = await request('/workspaces', { method: 'POST' });
  try {
    const features = Array.from({ length: count }, (_, i) => {
      const x = (i % 1000) * 3, y = Math.floor(i / 1000) * 3;
      return { type: 'Feature', id: -i - 2, properties: { ordinal: i }, geometry: family === 'point' ? { type: 'Point', coordinates: [x, y] } : { type: 'LineString', coordinates: [[x, y], [x + 1, y]] } };
    });
    const body = new FormData();
    body.append('file', new Blob([JSON.stringify({ type: 'FeatureCollection', features })]), `${family}.geojson`);
    const metrics = { family, features: count, requests: 0, maximum_ms: 0, import_ms: 0, validation_ms: 0 };
    let pending = true;
    let start = performance.now();
    const imported = request(`/workspaces/${workspace.id}/uploads`, { method: 'POST', body }).finally(() => { pending = false; });
    while (pending) { await health(metrics); await delay(20); }
    const ws = await imported;
    metrics.import_ms = performance.now() - start;
    assert.equal(ws.layers[0].feature_count, count);
    ws.config.checks = [{ rule: family === 'point' ? 'point/must-not-overlap' : 'line/must-not-overlap', layer: ws.layers[0].id, other: null, enabled: true }];
    await request(`/workspaces/${ws.id}/config`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(ws.config) });
    start = performance.now();
    await request(`/workspaces/${ws.id}/runs`, { method: 'POST' });
    for (;;) {
      await health(metrics);
      const status = await request(`/workspaces/${ws.id}/job`);
      if (status.job.status !== 'running') { assert.equal(status.job.status, 'completed', status.job.message); assert.equal(status.result.total, 0); break; }
      await delay(20);
    }
    metrics.validation_ms = performance.now() - start;
    assert(metrics.maximum_ms < 2000, 'HTTP responsiveness exceeded two seconds');
    console.log(JSON.stringify(metrics));
  } finally { await fetch(`${base}/workspaces/${workspace.id}`, { method: 'DELETE' }); }
}
