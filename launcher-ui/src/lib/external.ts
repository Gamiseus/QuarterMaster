import { openUrl } from "@tauri-apps/plugin-opener";
import { toasts } from "./stores/toasts.svelte";

export function isHttpsUrl(href: string | null | undefined): href is string {
  if (!href) return false;
  try {
    return new URL(href).protocol === "https:";
  } catch {
    return false;
  }
}

/** Opens an https URL in the default browser; anything else is ignored. */
export async function openExternal(href: string | null | undefined): Promise<void> {
  if (!isHttpsUrl(href)) return;
  try {
    // Normalised (lowercase scheme and host) so the opener's `https://*` scope matches.
    await openUrl(new URL(href).href);
  } catch (e) {
    toasts.error("Could not open the link.", e instanceof Error ? e.message : String(e));
  }
}

/**
 * Stops every link inside `node` from navigating the webview and opens https links externally.
 * Usable as a Svelte attachment; returns the cleanup.
 */
export function guardLinks(node: HTMLElement): () => void {
  const onClick = (e: MouseEvent) => {
    const link = (e.target as Element | null)?.closest?.("a[href]");
    if (!link || !node.contains(link)) return;
    e.preventDefault();
    if (e.type === "click") void openExternal(link.getAttribute("href"));
  };
  node.addEventListener("click", onClick);
  node.addEventListener("auxclick", onClick);
  return () => {
    node.removeEventListener("click", onClick);
    node.removeEventListener("auxclick", onClick);
  };
}
