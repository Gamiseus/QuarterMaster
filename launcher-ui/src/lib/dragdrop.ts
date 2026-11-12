import { getCurrentWebview } from "@tauri-apps/api/webview";
import { isMock } from "./events";

export interface FileDropEvent {
  type: "enter" | "leave" | "drop";
  paths: string[];
}

const MOCK_EVENT = "qm-drop";

declare global {
  interface Window {
    /** Mock mode only: simulates files dropped on the window (used by e2e specs). */
    __qmMockDrop?: (paths: string[]) => void;
    __qmMockDropBus?: EventTarget;
  }
}

function mockListen(handler: (e: FileDropEvent) => void): () => void {
  const bus = (window.__qmMockDropBus ??= new EventTarget());
  window.__qmMockDrop ??= (paths) =>
    bus.dispatchEvent(new CustomEvent<FileDropEvent>(MOCK_EVENT, { detail: { type: "drop", paths: [...paths] } }));
  const listener = (ev: Event) => handler((ev as CustomEvent<FileDropEvent>).detail);
  bus.addEventListener(MOCK_EVENT, listener);
  return () => bus.removeEventListener(MOCK_EVENT, listener);
}

/** Calls `handler` when files are dragged over, away from or dropped on the window. Returns the unsubscribe. */
export function onFileDrop(handler: (e: FileDropEvent) => void): () => void {
  if (isMock()) return mockListen(handler);
  // Outside the Tauri webview (unit tests) there is nothing to listen to.
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return () => {};
  let unlisten: (() => void) | null = null;
  let disposed = false;
  getCurrentWebview()
    .onDragDropEvent(({ payload }) => {
      if (payload.type === "over") return;
      handler({ type: payload.type, paths: payload.type === "leave" ? [] : payload.paths });
    })
    .then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    })
    .catch((e: unknown) => console.error("file drop listen failed", e));
  return () => {
    disposed = true;
    unlisten?.();
  };
}
