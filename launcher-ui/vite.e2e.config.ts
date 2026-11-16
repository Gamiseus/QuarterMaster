import { defineConfig, mergeConfig } from "vite";
import base from "./vite.config";

// The e2e server: the mock bundle built into dist-e2e (the app's dist stays untouched) and served
// by vite preview. "Connection: close" makes the server close each socket, so the TIME_WAIT
// sockets sit on port 5173 and not on Chromium's side; a run soon after another then does not
// fail page loads with net::ERR_ADDRESS_IN_USE (a blank page).
export default defineConfig((env) =>
  mergeConfig(base(env), {
    build: { outDir: "dist-e2e", emptyOutDir: true },
    preview: {
      host: "127.0.0.1",
      port: 5173,
      strictPort: true,
      headers: { Connection: "close" },
    },
  }),
);
