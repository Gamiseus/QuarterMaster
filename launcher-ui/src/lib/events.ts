import { listen } from "@tauri-apps/api/event";
import type { QmEvent } from "./bindings";
import { toasts } from "./stores/toasts.svelte";

export const EVENT_NAME = "qm://event";
const MOCK_EVENT = "qm";

declare global {
  interface Window {
    __qmMock?: EventTarget;
    __qmMockEmit?: (event: QmEvent) => void;
  }
}

export function isMock(): boolean {
  return import.meta.env.VITE_MOCK === "1";
}

function mockBus(): EventTarget {
  window.__qmMock ??= new EventTarget();
  return window.__qmMock;
}

/** Dispatches an event to `onEvent` handlers in mock mode. */
export function emitMock(event: QmEvent): void {
  mockBus().dispatchEvent(new CustomEvent<QmEvent>(MOCK_EVENT, { detail: event }));
}

/** Subscribes to backend events. Returns an unsubscribe function. */
export function onEvent(handler: (e: QmEvent) => void): () => void {
  if (isMock()) {
    const bus = mockBus();
    const listener = (ev: Event) => handler((ev as CustomEvent<QmEvent>).detail);
    bus.addEventListener(MOCK_EVENT, listener);
    return () => bus.removeEventListener(MOCK_EVENT, listener);
  }
  let unlisten: (() => void) | null = null;
  let disposed = false;
  listen<QmEvent>(EVENT_NAME, (e) => handler(e.payload))
    .then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    })
    .catch((e: unknown) => {
      console.error("event listen failed", e);
      toasts.error("Live updates are unavailable. Restart QuarterMaster.", e instanceof Error ? e.message : String(e));
    });
  return () => {
    disposed = true;
    unlisten?.();
  };
}
