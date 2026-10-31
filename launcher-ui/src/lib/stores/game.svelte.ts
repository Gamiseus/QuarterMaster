import { call } from "../api";
import { commands, type GameStatus, type QmEvent } from "../bindings";

const IDLE: GameStatus = { stage: { stage: "idle" }, profile_id: null, is_modded: false };

class GameStore {
  status = $state<GameStatus>(IDLE);
  /** Message from a `game_running` error; cleared on idle, on a non-busy load, or by the user. */
  banner = $state<string | null>(null);
  #seq = 0;

  get isBusy(): boolean {
    const s = this.status.stage.stage;
    return s !== "idle" && s !== "failed";
  }

  get isRunning(): boolean {
    return this.status.stage.stage === "running";
  }

  async load(): Promise<void> {
    const seq = ++this.#seq;
    const status = await call(commands.gameStatus());
    if (seq !== this.#seq) return;
    this.#set(status);
    if (!this.isBusy) this.banner = null;
  }

  async launch(profileId: string): Promise<void> {
    const status = await call(commands.gameLaunch({ profile_id: profileId, method: null }));
    this.#seq++;
    this.#set(status);
  }

  dismissBanner(): void {
    this.banner = null;
  }

  handle(e: QmEvent): void {
    if (e.type !== "game_status_changed") return;
    this.#seq++; // an in-flight load is older than this event
    this.#set(e.status);
  }

  #set(status: GameStatus): void {
    this.status = status;
    if (status.stage.stage === "idle") this.banner = null;
  }
}

export const game = new GameStore();
