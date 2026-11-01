import { mount } from "svelte";
import App from "./App.svelte";
import "./styles/fonts.css";
import "./styles/tokens.css";
import "./styles/base.css";

if (import.meta.env.VITE_MOCK === "1") {
  const { installMockBackend } = await import("./lib/mock/backend");
  installMockBackend();
}

const target = document.getElementById("app");
if (!target) throw new Error("missing #app");
mount(App, { target });
