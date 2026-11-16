import type { GameStatus, LaunchRequest, StartupStatus } from "../../bindings";
import { appInfo } from "../fixtures";
import { fail, isGameBusy, jobs, log, setGame, state, type Handlers } from "../state";

const startup: StartupStatus = { done: true, recovered: null, attached_pid: null };

/** Simulated launch: deploy, wait for Steam, run. */
function launch(req: LaunchRequest): GameStatus {
  if (isGameBusy()) fail("game_running", "The game is already running.");
  log.launches.push({ ...req });
  const modded = req.profile_id !== "vanilla" && !req.without_mods;
  const status: GameStatus = { stage: { stage: "deploying" }, profile_id: req.profile_id, is_modded: modded };
  state.game = status;
  setTimeout(() => setGame({ ...status, stage: { stage: "waiting_for_steam" } }), 600);
  setTimeout(() => setGame({ ...status, stage: { stage: "running", pid: 4242 } }), 1400);
  return status;
}

/** Next queued answer from `__qmMockInit.dialog`, else a fixed path for the dialog kind. */
function dialogOpen(options: { directory?: boolean } | undefined): string | null {
  const queue = state.init.dialog;
  if (queue && queue.length > 0) return queue.shift() ?? null;
  return options?.directory ? "C:\\Mods\\NewMod" : "C:\\Games\\Mud and Blood\\mnb.exe";
}

export const handlers: Handlers = {
  app_info: () => appInfo,
  startup_status: () => startup,
  game_status: () => state.game,
  game_launch: (args) => launch(args.req as LaunchRequest),
  jobs_list: () => jobs,
  "plugin:opener|open_url": () => null,
  "plugin:dialog|open": (args) => dialogOpen(args.options),
};
