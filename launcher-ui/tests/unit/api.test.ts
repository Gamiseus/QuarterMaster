import { describe, expect, it } from "vitest";
import { call, QmCommandError } from "../../src/lib/api";
import { game } from "../../src/lib/stores/game.svelte";
import { toasts } from "../../src/lib/stores/toasts.svelte";

describe("call", () => {
  it("returns data on ok", async () => {
    const data = await call(Promise.resolve({ status: "ok" as const, data: 42 }));
    expect(data).toBe(42);
  });

  it("throws QmCommandError with the error kind", async () => {
    const p = call(
      Promise.resolve({
        status: "error" as const,
        error: { kind: "game_running" as const, message: "The game is running.", details: null },
      }),
    );
    await expect(p).rejects.toBeInstanceOf(QmCommandError);
    await expect(p).rejects.toMatchObject({ kind: "game_running", details: null });
    expect(game.banner).toBe("The game is running.");
  });

  it("routes network errors to a toast with Retry", async () => {
    let retried = 0;
    const p = call(
      Promise.resolve({
        status: "error" as const,
        error: { kind: "network" as const, message: "Offline.", details: null },
      }),
      { retry: () => void retried++ },
    );
    await expect(p).rejects.toMatchObject({ kind: "network" });
    const toast = toasts.items.at(-1);
    expect(toast?.action?.label).toBe("Retry");
    toast?.action?.run();
    expect(retried).toBe(1);
  });

  it("routes other errors to a toast with details", async () => {
    const p = call(
      Promise.resolve({
        status: "error" as const,
        error: { kind: "io" as const, message: "Write failed.", details: "access denied" },
      }),
    );
    await expect(p).rejects.toMatchObject({ kind: "io" });
    const toast = toasts.items.at(-1);
    expect(toast?.message).toBe("Write failed.");
    expect(toast?.details).toBe("access denied");
  });

  it("wraps a non-QmError rejection as internal", async () => {
    const p = call(Promise.resolve({ status: "error" as const, error: "denied by ACL" as never }));
    await expect(p).rejects.toMatchObject({ kind: "internal", message: "Unexpected error.", details: "denied by ACL" });
    const toast = toasts.items.at(-1);
    expect(toast?.message).toBe("Unexpected error.");
    expect(toast?.details).toBe("denied by ACL");
  });
});
