<script setup lang="ts">
import { computed, ref } from "vue";
import { compatible, useWorkspace } from "../store";
import type { Check, GeometryKind } from "../types";
const store = useWorkspace();
const chosen = ref("line/must-not-have-dangles");
const manualCrs = ref("EPSG:28992");
const sourceCrs = computed(() => [
  ...new Set(
    store.ws?.layers
      .map((l) => l.source_crs)
      .filter((s): s is string => Boolean(s)),
  ),
]);
const available = computed(() =>
  store.rules.filter((r) =>
    candidates(r.primary).some(
      (a) => !r.secondary || candidates(r.secondary).some((b) => b.id !== a.id),
    ),
  ),
);
function candidates(kind: GeometryKind) {
  return (
    store.ws?.layers.filter(
      (l) =>
        store.draft?.layers.some((s) => s.id === l.id && s.selected) &&
        compatible(l.kinds, kind),
    ) ?? []
  );
}
function add() {
  const rule =
    available.value.find((r) => r.name === chosen.value) ?? available.value[0];
  if (!rule || !store.draft) return;
  const layer = candidates(rule.primary)[0]!.id;
  const other = rule.secondary
    ? candidates(rule.secondary).find((l) => l.id !== layer)?.id
    : null;
  if (rule.secondary && !other) {
    store.error = "This binary rule needs two distinct compatible layers";
    return;
  }
  store.draft.checks.push({
    rule: rule.name,
    layer,
    other: other ?? null,
    enabled: true,
  });
  store.dirty = true;
}
function ruleFor(check: Check) {
  return store.rules.find((r) => r.name === check.rule)!;
}
function setCrs(crs: string | null) {
  if (store.draft) {
    store.draft.working_crs = crs;
    store.dirty = true;
  }
}
</script>
<template>
  <aside class="rule-panel" v-if="store.draft">
    <div class="panel-title">
      <h2>Check configuration</h2>
      <span class="tiny-label">PLANAR</span>
    </div>
    <section class="config-section">
      <div class="section-label"><span>01</span> Coordinates</div>
      <label
        >Working CRS<select
          aria-label="Working CRS"
          :disabled="store.busy"
          :value="store.draft.working_crs ?? ''"
          @change="setCrs(($event.target as HTMLSelectElement).value || null)"
        >
          <option value="">Raw XY (unreferenced)</option>
          <option v-for="(crs, i) in sourceCrs" :key="crs" :value="crs">
            Imported CRS {{ i + 1 }}
          </option>
          <option
            v-if="
              store.draft.working_crs &&
              !sourceCrs.includes(store.draft.working_crs)
            "
            :value="store.draft.working_crs"
          >
            {{
              store.draft.working_crs.startsWith("EPSG:")
                ? store.draft.working_crs
                : "Custom CRS"
            }}
          </option>
        </select></label
      >
      <div class="crs-entry">
        <input
          v-model="manualCrs"
          aria-label="EPSG code"
          :disabled="store.busy"
          placeholder="EPSG:28992"
        /><button
          class="secondary"
          :disabled="store.busy"
          @click="setCrs(manualCrs.trim())"
        >
          Use CRS
        </button>
      </div>
      <label
        >Tolerance
        <span class="label-detail">{{
          store.ws?.working_crs_info?.units ?? "raw XY units"
        }}</span
        ><input
          type="number"
          min="0"
          step="any"
          v-model.number="store.draft.tolerance"
          @input="store.dirty = true"
          aria-label="Tolerance"
          :disabled="store.busy"
      /></label>
      <p class="field-note">
        Changing CRS restores its default tolerance. Calculations use complete
        source geometries.
      </p>
    </section>
    <section class="config-section rules-section">
      <div class="section-label">
        <span>02</span> Topology rules
        <b>{{ store.draft.checks.filter((c) => c.enabled).length }}</b>
      </div>
      <div class="rule-add">
        <select
          v-model="chosen"
          aria-label="Rule to add"
          :disabled="store.busy"
        >
          <option v-for="r in available" :key="r.name" :value="r.name">
            {{ r.name }}
          </option>
          <option v-if="!available.length" value="">
            Import compatible layers first
          </option></select
        ><button
          class="secondary"
          :disabled="store.busy || !available.length"
          @click="add"
        >
          + Add rule
        </button>
      </div>
      <p v-if="!store.draft.checks.length" class="empty-rules">
        Choose a rule to define what good topology looks like for your data.
      </p>
      <article
        v-for="(check, i) in store.draft.checks"
        :key="i"
        class="rule-card"
        :class="{ disabled: !check.enabled }"
      >
        <div class="rule-card-title">
          <input
            type="checkbox"
            v-model="check.enabled"
            @change="store.dirty = true"
            :aria-label="`Enable ${check.rule}`"
            :disabled="store.busy"
          />
          <h3>{{ check.rule.split("/")[1]?.replaceAll("-", " ") }}</h3>
          <button
            class="icon-button"
            :aria-label="`Remove rule ${i + 1}`"
            :disabled="store.busy"
            @click="
              store.draft.checks.splice(i, 1);
              store.dirty = true;
            "
          >
            ×
          </button>
        </div>
        <span class="rule-family">{{ check.rule.split("/")[0] }}</span>
        <p>{{ ruleFor(check)?.description }}</p>
        <label
          >Primary layer<select
            v-model="check.layer"
            @change="store.dirty = true"
            :aria-label="`Primary layer for rule ${i + 1}`"
            :disabled="store.busy"
          >
            <option
              v-for="l in candidates(ruleFor(check)?.primary ?? 'any')"
              :key="l.id"
              :value="l.id"
            >
              {{ l.name }}
            </option>
          </select></label
        ><label v-if="ruleFor(check)?.secondary"
          >Secondary layer<select
            v-model="check.other"
            @change="store.dirty = true"
            :aria-label="`Secondary layer for rule ${i + 1}`"
            :disabled="store.busy"
          >
            <option
              v-for="l in candidates(ruleFor(check).secondary!).filter(
                (l) => l.id !== check.layer,
              )"
              :key="l.id"
              :value="l.id"
            >
              {{ l.name }}
            </option>
          </select></label
        >
      </article>
    </section>
    <div class="run-footer">
      <button
        class="primary run-button"
        :disabled="
          store.busy ||
          store.ws?.job?.status === 'running' ||
          !store.draft.checks.some((c) => c.enabled)
        "
        @click="store.run"
      >
        {{
          store.ws?.job?.status === "running"
            ? "Checking complete dataset…"
            : "Run topology checks"
        }}<span v-if="store.ws?.job?.status !== 'running'">→</span></button
      ><button
        class="text-button"
        :disabled="!store.dirty || store.busy"
        @click="store.apply()"
      >
        {{ store.dirty ? "Apply configuration" : "Configuration saved" }}
      </button>
    </div>
  </aside>
</template>
