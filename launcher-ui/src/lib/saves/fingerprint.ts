/** Mod id to version, as stored in `SaveProfile.active_mods_fingerprint`. */
export type Fingerprint = Partial<Record<string, string>>;

export interface FingerprintDiff {
  added: string[];
  removed: string[];
  changed: { id: string; from: string; to: string }[];
}

/** What changes going from `a` to `b`; every list sorted by mod id. */
export function diffFingerprints(a: Fingerprint, b: Fingerprint): FingerprintDiff {
  const ids = (f: Fingerprint) => Object.keys(f).filter((k) => f[k] !== undefined);
  const added = ids(b).filter((id) => a[id] === undefined).sort();
  const removed = ids(a).filter((id) => b[id] === undefined).sort();
  const changed = ids(a)
    .filter((id) => b[id] !== undefined && a[id] !== b[id])
    .sort()
    .map((id) => ({ id, from: a[id] as string, to: b[id] as string }));
  return { added, removed, changed };
}
