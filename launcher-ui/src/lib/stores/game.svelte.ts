import { call } from "../api";
import { commands, type GameStatus, type LaunchMethod, type QmEvent, type TrialRequest } from "../bindings";

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

  /** `method` null uses the profile's own launch method; `withoutMods` starts the game with no mods. */
  async launch(profileId: string, method: LaunchMethod | null = null, withoutMods = false): Promise<void> {
    const req = withoutMods ? { profile_id: profileId, method, without_mods: true } : { profile_id: profileId, method };
    const status = await call(commands.gameLaunch(req));
    this.#seq++;
    this.#set(status);
  }

  /** Starts one test run of the active profile (D68); the profile itself is not changed. Refusals are rethrown for the dialog. */
  async launchTrial(req: TrialRequest): Promise<void> {
    const status = await call(commands.gameLaunchTrial(req), { expect: ["conflict", "io", "invalid_input"] });
    this.#seq++;
    this.#set(status);
  }

  /** Starts the next test of the search for a crash's cause (D68 step 2). Refusals are rethrown for the dialog. */
  async launchBisect(method: LaunchMethod | null = null): Promise<void> {
    const status = await call(commands.bisectLaunch(method), { expect: ["conflict", "io", "invalid_input", "not_found"] });
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
