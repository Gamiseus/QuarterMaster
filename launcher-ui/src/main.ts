import { mount } from "svelte";
import App from "./App.svelte";
import "./styles/fonts.css";
import "./styles/tokens.css";
import "./styles/cvd.css";
import "./styles/base.css";

const target = document.getElementById("app");
if (!target) throw new Error("missing #app");

if (import.meta.env.VITE_MOCK === "1") {
  const { installMockBackend } = await import("./lib/mock/backend");
  installMockBackend();
}

// Mock builds only: `/?harness=diff` shows the diff view alone, for the e2e performance test.
if (import.meta.env.VITE_MOCK === "1" && new URLSearchParams(location.search).get("harness") === "diff") {
  const { default: DiffHarness } = await import("./lib/mock/DiffHarness.svelte");
  mount(DiffHarness, { target });
} else {
  mount(App, { target });
}
