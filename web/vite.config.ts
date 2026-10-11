import { defineConfig } from "vitest/config";
import vue from "@vitejs/plugin-vue";
import tailwind from "@tailwindcss/vite";
export default defineConfig({
  plugins: [vue(), tailwind()],
  server: {
    port: 5173,
    strictPort: true,
    proxy: { "/api": "http://127.0.0.1:8080" },
  },
  build: {
    // Emits .vite/license.md with the bundled packages' licenses for releases.
    license: true,
    rollupOptions: {
      output: {
        manualChunks: {
          maps: ["ol/Map", "ol/View", "ol/format/GeoJSON"],
          controls: ["reka-ui"],
        },
      },
    },
  },
  test: { environment: "jsdom", include: ["tests/**/*.test.ts"] },
});
