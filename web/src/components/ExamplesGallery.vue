<script setup lang="ts">
import {
  DialogRoot,
  DialogPortal,
  DialogOverlay,
  DialogContent,
  DialogTitle,
  DialogDescription,
} from "reka-ui";
import { useWorkspace } from "../store";
const store = useWorkspace();
</script>
<template>
  <DialogRoot
    :open="store.galleryOpen"
    @update:open="store.galleryOpen = $event"
    ><DialogPortal
      ><DialogOverlay class="examples-overlay" /><DialogContent
        class="examples-gallery"
      >
        <div class="panel-title">
          <DialogTitle as="h2">Examples</DialogTitle>
          <button
            class="icon-button"
            aria-label="Close examples"
            @click="store.galleryOpen = false"
          >
            ×
          </button>
        </div>
        <DialogDescription>
          Explore real data from Natural Earth. Each example opens a fresh
          workspace. Click Run topology checks when you are ready.
        </DialogDescription>
        <p v-if="store.error" role="alert">{{ store.error }}</p>
        <p v-if="!store.examples.length && store.busy" role="status">
          Loading examples…
        </p>
        <button
          v-if="!store.examples.length && !store.busy"
          class="secondary"
          @click="store.openExamples"
        >
          Retry loading examples
        </button>
        <div class="examples-grid">
          <article
            v-for="example in store.examples"
            :key="example.id"
            class="example-card"
          >
            <h3>{{ example.title }}</h3>
            <p>{{ example.description }}</p>
            <div class="geometry-badges">
              <span
                v-for="kind in [
                  ...new Set(example.datasets.flatMap((d) => d.geometry_types)),
                ]"
                :key="kind"
                class="count-badge"
                >{{ kind }}</span
              >
            </div>
            <div
              v-for="dataset in example.datasets"
              :key="dataset.filename"
              class="example-source"
            >
              <span
                >{{ dataset.feature_count.toLocaleString() }} features<span
                  v-if="
                    dataset.imported_feature_count !== dataset.feature_count
                  "
                >
                  ({{ dataset.imported_feature_count }} imported)</span
                >
                · {{ dataset.filename }}</span
              >
              <a
                v-for="url in dataset.provenance.source_urls"
                :key="url"
                :href="url"
                target="_blank"
                rel="noopener noreferrer"
                >Source: {{ url.split("/").at(-1) }}</a
              >
            </div>
            <a
              :href="example.datasets[0]!.provenance.license_url"
              target="_blank"
              rel="noopener noreferrer"
              >Natural Earth · Public domain</a
            >
            <p v-if="example.note" class="example-note">{{ example.note }}</p>
            <button
              class="primary"
              :disabled="store.workspaceBusy"
              @click="store.loadExample(example.id)"
            >
              Open {{ example.title }}
            </button>
          </article>
        </div>
        <div class="tutorial-link">
          <p>
            A small synthetic street network demonstrates deliberately
            introduced topology issues.
          </p>
          <button
            class="text-button"
            :disabled="store.workspaceBusy"
            @click="store.loadExample()"
          >
            Synthetic topology tutorial
          </button>
        </div>
      </DialogContent></DialogPortal
    ></DialogRoot
  >
</template>
