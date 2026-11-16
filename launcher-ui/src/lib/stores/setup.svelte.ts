import { call } from "../api";
import { commands, type LegacyData, type ManualImportResult, type ManualInstall } from "../bindings";

/**
 * What the first start may find: data an earlier QuarterMaster left in AppData, and a manual
 * install in the game folder. Also whether the "Report a problem" dialog is open, since several
 * screens open it.
 */
class SetupStore {
  legacy = $state<LegacyData | null>(null);
  /** False until the AppData check answered; the first-run dialog waits for it. */
  legacyChecked = $state(false);
  manual = $state<ManualInstall | null>(null);
  /** "Not now": asked again at the next start. */
  manualDismissed = $state(false);
  reportOpen = $state(false);
  #manualSeq = 0;

  async checkLegacy(): Promise<void> {
    try {
      this.legacy = await call(commands.legacyDataCheck());
    } catch {
      this.legacy = null;
    } finally {
      this.legacyChecked = true;
    }
  }

  /** Moves the AppData data here and restarts the launcher on it. */
  async moveLegacy(): Promise<void> {
    await call(commands.legacyDataMove());
    this.legacy = null;
    await call(commands.appRestart());
  }

  declineLegacy(): void {
    this.legacy = null;
  }

  async checkManual(): Promise<void> {
    const seq = ++this.#manualSeq;
    let found: ManualInstall | null = null;
    try {
      found = await call(commands.manualInstallCheck());
    } catch {
      // call() reported it; nothing to offer.
    }
    if (seq === this.#manualSeq) this.manual = found;
  }

  async importManual(): Promise<ManualImportResult> {
    const result = await call(commands.manualInstallImport());
    this.#manualSeq++;
    return result;
  }

  closeManual(): void {
    this.manual = null;
    this.manualDismissed = true;
  }
}

export const setup = new SetupStore();
