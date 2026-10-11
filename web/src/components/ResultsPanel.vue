<script setup lang="ts">
import { useWorkspace } from "../store";
const store = useWorkspace();
</script>
<template>
  <section class="results-panel" aria-label="Validation results">
    <div class="results-header">
      <div class="result-heading">
        <h2>Inspection results</h2>
        <span class="result-count" v-if="store.ws?.result"
          >{{ store.ws.result.total.toLocaleString() }}
          {{ store.ws.result.total === 1 ? "violation" : "violations" }}</span
        ><span v-else class="tiny-label">READY WHEN YOU ARE</span>
      </div>
      <span class="snapshot-label" v-if="store.ws?.result"
        >Run revision {{ store.ws.result.revision }} · tolerance
        {{ store.ws.result.config.tolerance }}</span
      >
    </div>
    <div v-if="store.ws?.job?.status === 'running'" class="results-empty">
      <span class="spinner"></span>
      <h3>Checking every feature</h3>
      <p>You can continue exploring the preview while validation runs.</p>
    </div>
    <div v-else-if="!store.ws?.result" class="results-empty">
      <span class="results-symbol">◎</span>
      <h3>
        {{
          store.ws?.job?.status === "failed"
            ? "Validation did not finish"
            : "Every issue, in context"
        }}
      </h3>
      <p>
        {{
          store.ws?.job?.message ||
          "Configure your rules and run a check. Select any result to inspect its geometry and source features."
        }}
      </p>
    </div>
    <template v-else>
      <div class="rule-totals">
        <div
          v-for="(rule, i) in store.ws.result.rules"
          :key="i"
          class="rule-total"
        >
          <strong>{{ rule.rule.split("/")[1]?.replaceAll("-", " ") }}</strong
          ><span v-if="!rule.total" class="clean">✓ Clean</span
          ><span v-for="(count, category) in rule.counts" :key="category"
            >{{ category }} <b>{{ count.toLocaleString() }}</b></span
          >
        </div>
      </div>
      <details v-if="store.ws.result.warnings.length" class="result-warnings">
        <summary>
          {{ store.ws.result.warnings.length }} import and validation warnings
        </summary>
        <p v-for="(warning, i) in store.ws.result.warnings" :key="i">
          {{ warning }}
        </p>
      </details>
      <div v-if="store.ws.result.valid" class="clean-result">
        <span>✓</span>
        <div>
          <h3>All configured checks passed</h3>
          <p>The complete dataset has no violations for this configuration.</p>
        </div>
      </div>
      <div v-else class="results-table-wrap">
        <table>
          <thead>
            <tr>
              <th>Issue</th>
              <th>Rule</th>
              <th>Origin FID</th>
              <th>Destination FID</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="row in store.page?.violations"
              :key="row.id"
              :class="{ selected: store.selected?.id === row.id }"
              tabindex="0"
              @click="store.select(row)"
              @keydown.enter="store.select(row)"
            >
              <td><span class="issue-dot"></span>{{ row.category }}</td>
              <td :title="row.rule">
                {{ row.rule.split("/")[1]?.replaceAll("-", " ") }}
              </td>
              <td class="fid" :title="row.origin?.name">
                {{ row.origin?.fid ?? "Layer-wide" }}
              </td>
              <td class="fid" :title="row.destination?.name">
                {{ row.destination?.fid ?? "None" }}
              </td>
              <td>
                <button
                  class="text-button inspect-button"
                  :aria-label="`Inspect violation ${row.id + 1}`"
                  @click.stop="store.select(row)"
                >
                  Inspect ↗
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <div v-if="store.page && store.page.total > 0" class="pagination">
        <span
          >{{ store.page.page * 100 + 1 }}–{{
            Math.min(
              (store.page.page + 1) * 100,
              store.page.total,
            ).toLocaleString()
          }}
          of {{ store.page.total.toLocaleString() }}</span
        >
        <div>
          <button
            class="secondary"
            :disabled="store.page.page === 0 || store.busy"
            @click="store.perform(() => store.fetchPage(store.page!.page - 1))"
          >
            Previous</button
          ><span>Page {{ store.page.page + 1 }}</span
          ><button
            class="secondary"
            :disabled="
              (store.page.page + 1) * 100 >= store.page.total || store.busy
            "
            @click="store.perform(() => store.fetchPage(store.page!.page + 1))"
          >
            Next
          </button>
        </div>
      </div>
    </template>
  </section>
</template>
