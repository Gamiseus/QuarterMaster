<script lang="ts">
  import { Dialog } from "bits-ui";
  import type { Snippet } from "svelte";
  import { IconClose } from "../icons";
  import IconButton from "./IconButton.svelte";

  interface Props {
    open?: boolean;
    title: string;
    description?: string;
    /** Action row, usually Buttons. */
    footer?: Snippet;
    corners?: boolean;
    /** "first" focuses the first control in the body on open; "content" focuses the dialog itself. Never Close. */
    initialFocus?: "first" | "content";
    /** "lg" widens the dialog to --qm-dialog-lg (clamped to the viewport). */
    size?: "md" | "lg";
    /** False keeps the dialog open: Escape, an outside click and Close do nothing (e.g. while a confirm runs). */
    dismissible?: boolean;
    children?: Snippet;
  }

  let {
    open = $bindable(false),
    title,
    description,
    footer,
    corners = true,
    initialFocus = "first",
    size = "md",
    dismissible = true,
    children,
  }: Props = $props();

  let body = $state<HTMLElement | null>(null);
  let content = $state<HTMLElement | null>(null);

  const FOCUSABLE =
    'a[href], input, select, textarea, button, [contenteditable]:not([contenteditable="false"]), [tabindex="0"]';

  function usable(el: HTMLElement): boolean {
    if (el.closest("[hidden], [inert]") || (el as HTMLInputElement).type === "hidden") return false;
    if ((el as HTMLButtonElement).disabled || el.getAttribute("aria-disabled") === "true") return false;
    return true;
  }

  /** Tab order inside the dialog, header included. */
  function tabbables(): HTMLElement[] {
    return [...(content?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? [])].filter((el) => usable(el) && el.tabIndex >= 0);
  }

  /** Keeps Tab and Shift+Tab inside and wraps both ways (the library's own loop lets Shift+Tab on the first control escape). */
  function onkeydown(e: KeyboardEvent) {
    if (e.key !== "Tab" || !content) return;
    const items = tabbables();
    const first = items[0];
    const last = items[items.length - 1];
    const active = document.activeElement;
    if (!first || !last) {
      e.preventDefault();
      content.focus();
    } else if (e.shiftKey && (active === first || active === content)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && (active === last || active === content)) {
      e.preventDefault();
      first.focus();
    }
  }

  /** While not dismissible, Escape and an outside click do nothing. */
  function holdOpen(e: Event) {
    if (!dismissible) e.preventDefault();
  }

  /** Open focus goes to a usable `[data-autofocus]` control anywhere in the dialog, else the first usable control in
   * the body (or the dialog itself); never to Close, whose tooltip would pop up. */
  function onOpenAutoFocus(e: Event) {
    e.preventDefault();
    const marked = [...(content?.querySelectorAll<HTMLElement>("[data-autofocus]") ?? [])].find(usable);
    const first =
      marked ??
      (initialFocus === "first"
        ? [...(body?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? [])].find(usable)
        : undefined);
    (first ?? content)?.focus();
    // A control that refused focus must not leave it outside the dialog.
    if (content && !content.contains(document.activeElement)) content.focus();
  }
</script>

<Dialog.Root
  bind:open={() => open, (v) => {
    if (v || dismissible) open = v;
  }}
>
  <Dialog.Portal>
    <Dialog.Overlay class="qm-dialog-overlay" />
    <Dialog.Content class={["qm-dialog", `qm-dialog-${size}`, { "qm-corners": corners }]} tabindex={-1} bind:ref={content}
      onEscapeKeydown={holdOpen}
      onInteractOutside={holdOpen}
      {onOpenAutoFocus}
      {onkeydown}
    >
      <header class="head">
        <Dialog.Title class="qm-dialog-title">{title}</Dialog.Title>
        <Dialog.Close>
          {#snippet child({ props })}
            <IconButton icon={IconClose} label="Close" {...props} disabled={!dismissible} />
          {/snippet}
        </Dialog.Close>
      </header>
      {#if description}
        <Dialog.Description class="qm-dialog-description">{description}</Dialog.Description>
      {/if}
      {#if children}
        <div class="body" bind:this={body}>{@render children()}</div>
      {/if}
      {#if footer}
        <footer class="footer">{@render footer()}</footer>
      {/if}
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<style>
  /* Dialog parts are portalled out of this component, so their rules are global. */
  :global(.qm-dialog-overlay) {
    position: fixed;
    inset: 0;
    z-index: var(--qm-z-overlay);
    background: color-mix(in srgb, var(--qm-canvas) 70%, transparent);
  }

  :global(.qm-dialog) {
    position: fixed;
    top: 50%;
    left: 50%;
    z-index: var(--qm-z-dialog);
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
    width: min(calc(var(--qm-detail-w) * 1.5), calc(100vw - var(--qm-space-6)));
    max-height: calc(100vh - var(--qm-space-6));
    padding: var(--qm-space-5);
    transform: translate(-50%, -50%);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-lg);
    background: var(--qm-popover);
    color: var(--qm-text);
  }

  :global(.qm-dialog.qm-dialog-lg) {
    width: min(var(--qm-dialog-lg), calc(100vw - var(--qm-space-6)));
  }

  :global(.qm-dialog:focus-visible) {
    box-shadow: none;
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-3);
  }

  :global(.qm-dialog-title) {
    margin: 0;
    font-family: var(--qm-font-stencil);
    font-size: var(--qm-text-16);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  :global(.qm-dialog-description) {
    margin: 0;
    color: var(--qm-text-muted);
    font-size: var(--qm-text-13);
  }

  .body {
    overflow: auto;
  }

  .footer {
    display: flex;
    justify-content: flex-end;
    gap: var(--qm-space-2);
  }
</style>
