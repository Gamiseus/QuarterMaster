/// <reference types="vitest/config" />
import { defaultClientConditions, defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig(({ mode }) => ({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  // Svelte 5 components must resolve to the browser build under Vitest; extend (never replace)
  // Vite's client defaults, or the production bundle would pick Svelte's server entry.
  resolve: mode === "test" ? { conditions: [...defaultClientConditions, "browser"] } : undefined,
  build: {
    target: "es2022",
  },
  test: {
    environment: "jsdom",
    include: ["tests/unit/**/*.test.ts"],
    setupFiles: ["tests/setup.ts"],
    // Let ?raw imports of the style sheets return real text (tokens.test.ts).
    css: { include: [/src\/styles\/.+\.css/] },
  },
}));
