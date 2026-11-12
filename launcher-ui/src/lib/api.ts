import type { ErrorKind, QmError } from "./bindings";
import { game } from "./stores/game.svelte";
import { toasts } from "./stores/toasts.svelte";

export class QmCommandError extends Error {
  kind: ErrorKind;
  details: string | null;

  constructor(error: QmError) {
    super(error.message);
    this.name = "QmCommandError";
    this.kind = error.kind;
    this.details = error.details;
  }
}

type CommandResult<T> = { status: "ok"; data: T } | { status: "error"; error: QmError };

export interface CallOptions {
  /** Re-runs the action; offered as Retry on network errors. */
  retry?: () => unknown;
  /** Errors of these kinds are rethrown without a toast or banner, for callers that show them inline. */
  expect?: ErrorKind[];
}

function route(err: QmCommandError, opts: CallOptions): void {
  if (opts.expect?.includes(err.kind)) return;
  if (err.kind === "game_running") {
    game.banner = err.message;
  } else if (err.kind === "network") {
    const retry = opts.retry;
    toasts.error(err.message, err.details, retry ? { label: "Retry", run: () => void retry() } : undefined);
  } else {
    toasts.error(err.message, err.details);
  }
}

/** Bindings pass any non-Error rejection through as `error` (e.g. a plain string from an ACL denial). */
function toQmError(e: unknown): QmError {
  if (typeof e === "object" && e !== null) {
    const o = e as Record<string, unknown>;
    if (typeof o.kind === "string" && typeof o.message === "string") {
      return {
        kind: o.kind as ErrorKind,
        message: o.message,
        details: typeof o.details === "string" ? o.details : null,
      };
    }
  }
  return { kind: "internal", message: "Unexpected error.", details: String(e) };
}

/** Unwraps a generated command result, routing errors to the banner or a toast. */
export async function call<T>(p: Promise<CommandResult<T>>, opts: CallOptions = {}): Promise<T> {
  let result: CommandResult<T>;
  try {
    result = await p;
  } catch (e) {
    const err = new QmCommandError({
      kind: "internal",
      message: "The launcher could not run that command.",
      details: e instanceof Error ? e.message : String(e),
    });
    route(err, opts);
    throw err;
  }
  if (result.status === "ok") return result.data;
  const err = new QmCommandError(toQmError(result.error));
  route(err, opts);
  throw err;
}
