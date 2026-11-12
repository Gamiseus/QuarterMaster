import type { ArchiveModCandidate, ImportRequest } from "../bindings";

/** What to do with a mod that is already installed (the backend's `ConflictPolicy`). */
export type Choice = "upgrade" | "replace" | "install_alongside";

export interface ChoiceItem {
  value: Choice;
  label: string;
  description: string;
  disabled?: boolean;
  disabledReason?: string;
}

/**
 * The choices for a candidate; none when the mod is not installed yet. For the installed version itself,
 * Install alongside is left out: it writes the same folder as Reinstall.
 */
export function choicesFor(c: ArchiveModCandidate, gameBusy: boolean): ChoiceItem[] {
  if (c.already_installed_version === null) return [];
  const same = c.already_installed_version === c.manifest.version;
  const items: ChoiceItem[] = [
    same
      ? { value: "upgrade", label: "Reinstall", description: "Install this version again over the installed copy." }
      : { value: "upgrade", label: "Upgrade", description: "Profiles switch to this version. Other versions stay installed." },
    {
      value: "replace",
      label: "Replace",
      description: "Remove every installed version, then install this one.",
      ...(gameBusy ? { disabled: true, disabledReason: "Close the game to replace a mod." } : {}),
    },
    {
      value: "install_alongside",
      label: "Install alongside",
      description: "Keep the installed versions. Profiles stay on the version they use.",
    },
  ];
  return same ? items.filter((i) => i.value !== "install_alongside") : items;
}

/**
 * One request per distinct choice, so each policy applies only to its own mods. Mods that are not
 * installed (`null`) join the upgrade request when there is one: a policy only matters for an installed mod.
 * Requests come in order of first appearance.
 */
export function buildRequests(path: string, allowUnmanaged: boolean, selected: Map<string, Choice | null>): ImportRequest[] {
  const hasUpgrade = [...selected.values()].includes("upgrade");
  const groups = new Map<Choice | null, string[]>();
  for (const [id, choice] of selected) {
    const key = choice === null && hasUpgrade ? "upgrade" : choice;
    groups.set(key, [...(groups.get(key) ?? []), id]);
  }
  return [...groups].map(([policy, ids]) => ({
    path,
    allow_unmanaged: allowUnmanaged,
    selected_mod_ids: ids,
    conflict_policy: policy,
  }));
}
