//! Compatibility mods: what a mod's overrides and anchors did, and "Save as compatibility
//! patch", which turns profile fixes into a mod (Phase C §8, §10, §11.6).
use crate::cache::Loadable;
use crate::dto::{
    CompatChangesView, CompatEquivalence, CompatExportPreview, CompatExportRequest,
    CompatExportResult, CompatFile, CompatRefusal, CompatRequirement,
};
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::Scope;
use crate::ops::patch_tools::MSG_NOT_READY;
use crate::ops::profiles::vanilla_profile_error;
use crate::service::Service;
use qm_core::handshake::{BindingStatus, LoadOrderPlan, OverrideReport};
use qm_core::lenient::Lenient;
use qm_core::patch::compat::{overrides_toml, Override};
use qm_core::patch::fingerprint::{PatchFingerprintLite, PatchRef};
use qm_core::patch::order::{name_matches, patch_name};
use qm_core::patch::plan::{self, BindKind, Fingerprints, FpMode, RawLoad};
use qm_core::patch::table::PatchTable;
use qm_core::patch::Patch;
use qm_manager::fixes::{self, FixChange, JournalEntry, ProfileFix, FIXES_VERSION};
use qm_manager::{ModProfile, SimulationReport, VANILLA_ID};
use qm_schema::ModManifest;
use std::collections::{BTreeSet, HashSet};

/// The version every exported mod gets.
const EXPORT_VERSION: &str = "1.0.0";
/// The longest mod id (`^[a-z0-9_]{3,48}$`).
const MAX_ID: usize = 48;
/// The longest mod name, in characters.
const MAX_NAME: usize = 64;

/// One fix as an override entry.
struct Part {
    fix: ProfileFix,
    entry: Override,
    /// The loaded id of the mod whose patch it changes.
    target: String,
    /// Every mod the entry refers to (the target first), with why.
    refs: Vec<(String, Why)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Why {
    Changes,
    PlacesNext,
    Orders,
    /// It keeps that mod's change to a line another patch also changes (D49).
    KeepsChange,
    /// It orders its block next to that mod's block at a shared line (D55).
    Stacks,
}

/// The export worked out: what the preview shows, and what the export writes.
pub(crate) struct ExportWork {
    pub preview: CompatExportPreview,
    /// The profile with the mod in place of the included fixes.
    pub profile: Option<ModProfile>,
    pub removed: Vec<ProfileFix>,
    /// The plan activation would write for `profile`.
    pub plan: Option<LoadOrderPlan>,
    /// The simulation with the mod installed and the fixes gone.
    pub report: Option<SimulationReport>,
}

impl Service {
    /// What `mod_id`'s `[[overrides]]` did in the cached simulation (applied, and everything
    /// else), and the rungs that are never used on its own patches or from its overrides
    /// (D33: a rung on a mod it does not require). `Loading` until the simulation is cached.
    pub fn compat_changes_get(&self, mod_id: &str) -> QmResult<Loadable<CompatChangesView>> {
        self.installed_mod(mod_id)?;
        let Some(report) = self.inner.caches.simulation.get() else {
            return Ok(Loadable::Loading);
        };
        let mine = |m: &str| m.eq_ignore_ascii_case(mod_id);
        let (applied, rejected): (Vec<OverrideReport>, Vec<OverrideReport>) = report
            .overrides
            .iter()
            .filter(|o| mine(&o.from_mod))
            .cloned()
            .partition(|o| o.status == "applied");
        // Rung reports name patches as `patch_name` does in the simulated table.
        let own: HashSet<String> = self
            .inner
            .caches
            .patch_table
            .get()
            .map(|t| {
                t.patches
                    .iter()
                    .enumerate()
                    .filter(|(_, (p, _, _))| p.mod_id().is_some_and(mine))
                    .map(|(i, (p, _, _))| patch_name(p, i))
                    .collect()
            })
            .unwrap_or_default();
        let from_overrides = format!("compat:{}", mod_id.to_lowercase());
        let rejected_rungs = report
            .invalid_rungs
            .iter()
            .filter(|r| own.contains(&r.patch_id) || r.source.eq_ignore_ascii_case(&from_overrides))
            .cloned()
            .collect();
        Ok(Loadable::Ready(CompatChangesView {
            applied,
            rejected,
            rejected_rungs,
        }))
    }

    /// What saving `req.fix_ids` as a compatibility patch would make, and whether the game
    /// code would stay the same with the mod installed and enabled and the fixes removed
    /// (§8.2). Never writes.
    pub fn compat_export_preview(&self, req: CompatExportRequest) -> QmResult<CompatExportPreview> {
        let raw = self.current_raw().ok_or_else(not_readable)?;
        let (p, _) = self
            .migrated_in_memory(self.active_profile()?, Some(&raw))
            .map_err(|why| {
                QmError::new(
                    ErrorKind::InvalidInput,
                    format!("Your existing fixes could not be imported yet: {why}"),
                )
            })?;
        Ok(self.export_work(&p, &raw, &req)?.preview)
    }

    /// Saves `req.fix_ids` as a compatibility patch (D14): only when the preview shows the
    /// same game code, installs the mod into the library, enables it in the active profile
    /// and removes the fixes, as one step `fix_journal_undo` reverts. The files are written
    /// under the app data folder only. All or nothing: a failure after the import uninstalls
    /// the mod and puts the profile back (L6).
    pub fn compat_export(&self, req: CompatExportRequest) -> QmResult<CompatExportResult> {
        self.ensure_not_running()?;
        self.ensure_no_install()?;
        let _install = self.lock_install();
        // The profile as on disk, put back if a step after the import fails.
        let before = self.active_profile()?;
        let (p, raw) = self.ledger_profile()?;
        let raw = raw.ok_or_else(not_readable)?;
        let work = self.export_work(&p, &raw, &req)?;
        let preview = work.preview;
        if !preview.refused.is_empty() {
            let why: Vec<String> = preview.refused.iter().map(|r| r.reason.clone()).collect();
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                format!(
                    "Some fixes cannot be saved as a compatibility patch: {}",
                    why.join(" ")
                ),
            ));
        }
        match &preview.equivalence {
            CompatEquivalence::Equivalent => {}
            CompatEquivalence::Differs { files } => {
                return Err(QmError::new(
                    ErrorKind::Conflict,
                    format!(
                        "The compatibility patch would change the game code in {}. Keep the fixes instead.",
                        files.join(", ")
                    ),
                ));
            }
            CompatEquivalence::NotChecked { reason } => {
                return Err(QmError::new(ErrorKind::InvalidInput, reason.clone()));
            }
        }
        let Some(q) = work.profile else {
            return Err(QmError::new(
                ErrorKind::Internal,
                "The export has no profile.",
            ));
        };
        // Checked again just before the import: the work above takes a while.
        if q.id == VANILLA_ID {
            return Err(vanilla_profile_error());
        }
        self.ensure_not_running()?;

        // A folder the backend picks, under the app data folder.
        let dir = self
            .paths()
            .root_dir
            .join("tmp")
            .join(format!("compat-{}", preview.mod_id));
        let written = write_files(&dir, &preview.files);
        let imported = written.and_then(|()| {
            self.paths()
                .import_mod_from_path(&dir, None, false)
                .map_err(|e| {
                    QmError::new(
                        ErrorKind::Io,
                        "The compatibility patch could not be installed.",
                    )
                    .with_details(e)
                })
        });
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            tracing::warn!("could not remove {}: {e}", dir.display());
        }
        let summary = imported?;
        self.invalidate(Scope::Mods);

        // From here every error undoes the import.
        let mut ids: Vec<String> = Vec::new();
        for m in &summary.imported_mods {
            if !ids.contains(&m.manifest.id) {
                ids.push(m.manifest.id.clone());
            }
        }
        let shown_path = summary
            .imported_mods
            .iter()
            .find(|m| m.manifest.id == preview.mod_id)
            .map(|m| m.install_path.display().to_string());
        let mut saved = false;
        match self.finish_export(q, &preview, &work.removed, shown_path, &mut saved) {
            Ok(done) => Ok(done),
            Err(e) => Err(self.undo_export(&ids, &before, saved, e)),
        }
    }

    /// The steps after the import: the journal entry, then `commit_active_profile` step by
    /// step, so `saved` tells the rollback whether profile.json was written.
    fn finish_export(
        &self,
        mut q: ModProfile,
        preview: &CompatExportPreview,
        removed: &[ProfileFix],
        shown_path: Option<String>,
        saved: &mut bool,
    ) -> QmResult<CompatExportResult> {
        let shown_path = shown_path.ok_or_else(|| {
            QmError::new(
                ErrorKind::Internal,
                "The compatibility patch was not installed under its id.",
            )
        })?;
        // Saved with the profile, so a rollback that puts the profile back drops it too.
        fixes::push_journal(
            &mut q,
            JournalEntry {
                at: jiff::Zoned::now().to_string(),
                op: "compat_export".to_string(),
                label: format!("Saved as compatibility patch \"{}\"", preview.name),
                removed: removed.iter().cloned().map(Lenient::Ok).collect(),
                added: Vec::new(),
                mods_enabled: vec![preview.mod_id.clone()],
            },
        );
        self.ensure_not_running()?;
        q.updated_at = jiff::Zoned::now().to_string();
        self.save_profile(&q).or_kind(ErrorKind::Io)?;
        *saved = true;
        self.stage_profile(&q.id)?;
        self.after_active_profile_change();
        Ok(CompatExportResult {
            mod_id: preview.mod_id.clone(),
            version: preview.version.clone(),
            shown_path,
            removed_fix_ids: removed.iter().map(|f| f.id.clone()).collect(),
        })
    }

    /// Undoes an export that failed after the import: deletes the library folders of `ids`,
    /// puts `before` back when the profile was saved, and returns `cause` saying what was
    /// undone. Other profiles are not touched (`uninstall_mod` would edit them all).
    fn undo_export(
        &self,
        ids: &[String],
        before: &ModProfile,
        saved: bool,
        cause: QmError,
    ) -> QmError {
        let mut failed: Vec<String> = Vec::new();
        for id in ids {
            let dir = self.paths().library_dir.join(id);
            let safe = !id.is_empty() && id != "." && id != ".." && !id.contains(['/', '\\', ':']);
            let removed = if safe {
                std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))
            } else {
                Err(format!("Invalid mod id '{id}'"))
            };
            if let Err(e) = removed {
                tracing::warn!("could not remove {id} after a failed export: {e}");
                failed.push(e);
            }
        }
        if saved {
            let put_back = self
                .save_profile(before)
                .or_kind(ErrorKind::Io)
                .and_then(|()| self.stage_profile(&before.id));
            if let Err(e) = put_back {
                tracing::warn!("could not put the profile back after a failed export: {e}");
                failed.push(e.message);
            }
        }
        self.invalidate(Scope::Mods);
        self.after_active_profile_change();

        let undone = if !failed.is_empty() {
            "The export could not be fully undone. Check the mod list and your fixes."
        } else if saved {
            "The export was undone: the compatibility patch was uninstalled and the profile was put back."
        } else {
            "The export was undone: the compatibility patch was uninstalled and your fixes were not changed."
        };
        let details: Vec<String> = cause.details.into_iter().chain(failed).collect();
        QmError {
            kind: cause.kind,
            message: format!("{} {undone}", cause.message),
            details: (!details.is_empty()).then(|| details.join("\n")),
        }
    }

    /// Works out the export of `req` from the migrated profile `p` and the mods as read.
    pub(crate) fn export_work(
        &self,
        p: &ModProfile,
        raw: &RawLoad,
        req: &CompatExportRequest,
    ) -> QmResult<ExportWork> {
        if req.fix_ids.is_empty() {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                "Pick at least one fix to save.",
            ));
        }
        let current = self
            .inner
            .caches
            .simulation
            .get()
            .ok_or_else(|| QmError::new(ErrorKind::Conflict, MSG_NOT_READY))?;
        let vanilla = self
            .inner
            .caches
            .vanilla
            .get()
            .ok_or_else(|| QmError::new(ErrorKind::Conflict, MSG_NOT_READY))?;

        let mut refused = Vec::new();
        let mut parts: Vec<Part> = Vec::new();
        let wanted: BTreeSet<&str> = req.fix_ids.iter().map(String::as_str).collect();
        if p.fixes_version < FIXES_VERSION {
            for id in &wanted {
                refused.push(refusal(id, "Import your earlier fixes first."));
            }
        } else {
            for id in &wanted {
                if fixes::find(p, id).is_none() {
                    refused.push(refusal(id, "That fix is already gone."));
                }
            }
            let mut fps = Fingerprints::new(raw, FpMode::All);
            let copies = copy_ids(p);
            for f in fixes::fixes(p).filter(|f| wanted.contains(f.id.as_str())) {
                match part_of(raw, &copies, &mut fps, f) {
                    Ok(part) => parts.push(part),
                    Err(why) => refused.push(refusal(&f.id, &why)),
                }
            }
        }

        let installed = self.installed_mods();
        let names = self.mod_names();
        let label = |m: &str| {
            names
                .get(m)
                .cloned()
                .or_else(|| meta_of(raw, m).and_then(|x| x.name.clone()))
                .unwrap_or_else(|| m.to_string())
        };
        let Some(first) = parts.first() else {
            let reason = "None of these fixes can be saved as a compatibility patch.".to_string();
            return Ok(ExportWork {
                preview: CompatExportPreview {
                    mod_id: String::new(),
                    name: String::new(),
                    version: EXPORT_VERSION.to_string(),
                    requires: Vec::new(),
                    load_after: Vec::new(),
                    files: Vec::new(),
                    equivalence: CompatEquivalence::NotChecked { reason },
                    included: Vec::new(),
                    refused,
                },
                profile: None,
                removed: Vec::new(),
                plan: None,
                report: None,
            });
        };

        // The mods it requires: the changed mods first, then the others, each with why.
        let mut requires: Vec<CompatRequirement> = Vec::new();
        for (m, why) in parts
            .iter()
            .flat_map(|x| x.refs.iter())
            .filter(|(_, w)| *w == Why::Changes)
            .chain(
                parts
                    .iter()
                    .flat_map(|x| x.refs.iter())
                    .filter(|(_, w)| *w != Why::Changes),
            )
        {
            let l = label(m);
            let reason = match why {
                Why::Changes => format!("it changes {l}'s patch"),
                Why::PlacesNext => format!("it places code next to {l}'s change"),
                Why::Orders => format!("it runs a patch before or after {l}'s patch"),
                Why::KeepsChange => format!("it keeps {l}'s change to a line"),
                Why::Stacks => format!("it puts its code next to {l}'s code at a shared line"),
            };
            if let Some(r) = requires
                .iter_mut()
                .find(|r| r.mod_id.eq_ignore_ascii_case(m))
            {
                if !r.reasons.contains(&reason) {
                    r.reasons.push(reason);
                }
                continue;
            }
            // The id as mod.json has it: the solver matches ids exactly.
            let mod_id = installed
                .iter()
                .find(|i| i.manifest.id.eq_ignore_ascii_case(m))
                .map(|i| i.manifest.id.clone())
                .unwrap_or_else(|| m.clone());
            let version = meta_of(raw, m).and_then(|x| x.version.clone());
            requires.push(CompatRequirement {
                mod_id,
                name: l,
                range: version
                    .as_deref()
                    .filter(|v| qm_schema::version::ParsedVersion::parse(v).is_ok())
                    .map_or_else(|| "*".to_string(), |v| format!(">={v}")),
                version,
                reasons: vec![reason],
            });
        }
        let load_after: Vec<String> = requires.iter().map(|r| r.mod_id.clone()).collect();

        // Ids any profile names are taken too: a new mod under such an id would show up in
        // that profile.
        let profiles = self.paths().list_profiles();
        let referenced = profiles.iter().flat_map(|p| {
            p.enabled_mods
                .iter()
                .chain(&p.locked_mods)
                .chain(&p.favorites)
                .chain(p.mod_priorities.keys())
                .chain(p.mod_notes.keys())
                .chain(p.pinned_versions.keys())
                .map(|m| m.to_lowercase())
        });
        let taken: HashSet<String> = installed
            .iter()
            .map(|i| i.manifest.id.to_lowercase())
            .chain(
                raw.mods
                    .iter()
                    .flat_map(|m| std::iter::once(m.id.clone()).chain(m.provides.iter().cloned())),
            )
            .chain(referenced)
            .collect();
        let other = requires
            .iter()
            .map(|r| r.mod_id.to_lowercase())
            .find(|m| *m != first.target);
        let mod_id = unique_id(&first.target, other.as_deref(), &taken);
        let name = clean_name(&req.name).unwrap_or_else(|| {
            let mods: Vec<&str> = requires.iter().map(|r| r.name.as_str()).take(2).collect();
            clip(&format!("Compatibility patch for {}", mods.join(" and ")))
        });

        let manifest = manifest_for(&mod_id, &name, &requires, &features_for(&parts, raw));
        let mod_json = serde_json::to_string_pretty(&manifest_json(&manifest)?)
            .map_err(|e| internal("Could not write mod.json.", e.to_string()))?;
        let qm_toml = toml_for(&name, &p.name, &parts)?;
        let readme = readme_for(&name, &p.name, &parts, &requires);

        // The mod as the loader would read it once installed, with no disk access.
        let raw2 = raw
            .with_extra_mod(&mod_json, &qm_toml)
            .map_err(|e| internal("The compatibility patch does not read back.", e))?;
        let read_back: Vec<&Override> = raw2
            .overrides
            .iter()
            .filter(|(ix, _)| raw2.mods.get(*ix as usize).is_some_and(|m| m.id == mod_id))
            .filter_map(|(_, o)| o.ok())
            .collect();
        let intended: Vec<&Override> = parts.iter().map(|x| &x.entry).collect();
        if read_back != intended {
            return Err(internal(
                "The compatibility patch does not read back.",
                "qm.toml gives other overrides".to_string(),
            ));
        }

        // The profile after the export, planned as activation would plan it (§8.2).
        let mut q = p.clone();
        let mut removed = Vec::new();
        for x in &parts {
            removed.extend(fixes::remove(&mut q, &x.fix.id));
        }
        fixes::sync_legacy(&mut q);
        if !q.enabled_mods.contains(&mod_id) {
            q.enabled_mods.push(mod_id.clone());
        }
        let mut manifests: Vec<ModManifest> =
            installed.iter().map(|i| i.manifest.clone()).collect();
        manifests.push(
            ModManifest::from_json_str(&mod_json)
                .map_err(|e| internal("The compatibility patch does not read back.", e))?,
        );
        let plan2 = qm_manager::build_plan_for(&q, &manifests);
        // Through JSON, as the game reads the plan activation writes.
        let json = serde_json::to_string(&plan2)
            .map_err(|e| internal("Could not compile the preview.", e.to_string()))?;
        let applied = plan::apply_plan_with(&raw2, &LoadOrderPlan::parse(&json), FpMode::All);
        let table = PatchTable::from_applied(applied, raw2.mod_dir.clone());
        let report = self.simulate_table(&table, &vanilla);
        let differs = differing_files(&current, &report, &vanilla.files);
        let equivalence = if differs.is_empty() {
            CompatEquivalence::Equivalent
        } else {
            CompatEquivalence::Differs { files: differs }
        };

        Ok(ExportWork {
            preview: CompatExportPreview {
                mod_id,
                name,
                version: EXPORT_VERSION.to_string(),
                requires,
                load_after,
                files: vec![
                    CompatFile {
                        name: "mod.json".to_string(),
                        text: mod_json,
                    },
                    CompatFile {
                        name: "qm.toml".to_string(),
                        text: qm_toml,
                    },
                    CompatFile {
                        name: "README.txt".to_string(),
                        text: readme,
                    },
                ],
                equivalence,
                included: parts.iter().map(|x| x.fix.id.clone()).collect(),
                refused,
            },
            profile: Some(q),
            removed,
            plan: Some(plan2),
            report: Some(report),
        })
    }
}

fn not_readable() -> QmError {
    QmError::new(ErrorKind::Conflict, "The profile's mods could not be read.")
}

fn internal(msg: &str, details: String) -> QmError {
    QmError::new(ErrorKind::Internal, msg).with_details(details)
}

fn refusal(fix_id: &str, reason: &str) -> CompatRefusal {
    CompatRefusal {
        fix_id: fix_id.to_string(),
        reason: reason.to_string(),
    }
}

fn meta_of<'a>(raw: &'a RawLoad, m: &str) -> Option<&'a plan::ModMeta> {
    raw.mods.iter().find(|x| x.id.eq_ignore_ascii_case(m))
}

/// The loaded mod `name` means: by id, else the first that provides it.
fn loaded_mod(raw: &RawLoad, name: &str) -> Option<String> {
    let n = name.trim().to_lowercase();
    raw.mods
        .iter()
        .find(|m| m.id == n)
        .or_else(|| raw.mods.iter().find(|m| m.provides.contains(&n)))
        .map(|m| m.id.clone())
}

/// The ids of the profile's adjusted copies, running or paused, lowercase.
fn copy_ids(p: &ModProfile) -> HashSet<String> {
    fixes::fixes(p)
        .filter_map(|f| match &f.change {
            FixChange::AdjustedCopy { glue, .. } => Some(glue.id.to_lowercase()),
            _ => None,
        })
        .chain(p.glue_patches.iter().map(|g| g.id.to_lowercase()))
        .collect()
}

/// The mods a rule or rung name refers to: the mods of the patches it matches, else its
/// mod part. Err names a mod that is off, or an adjusted copy (`copies`): a copy belongs to
/// the profile, whichever mod it runs as (D58).
fn name_mods(raw: &RawLoad, copies: &HashSet<String>, name: &str) -> Result<Vec<String>, String> {
    let bare = name.trim().trim_start_matches('=');
    let lower = bare.to_lowercase();
    if copies.contains(&lower)
        || lower
            .split_once(':')
            .is_some_and(|(_, rest)| copies.contains(rest))
    {
        return Err(COPY_NAMED.to_string());
    }
    let mod_part = |m: &str| -> Result<Vec<String>, String> {
        if m.eq_ignore_ascii_case(plan::GLUE_MOD) {
            return Err(COPY_NAMED.to_string());
        }
        loaded_mod(raw, m)
            .map(|x| vec![x])
            .ok_or_else(|| format!("It refers to {m}, which is off. Turn it on first."))
    };
    if let Some(m) = bare.strip_suffix(":*") {
        return mod_part(m);
    }
    let hits: BTreeSet<String> = fixes::hits(raw, name, true)
        .into_iter()
        .filter_map(|i| raw.patches.get(i))
        .filter_map(|rp| raw.mods.get(rp.mod_ix as usize))
        .map(|m| m.id.clone())
        .collect();
    if !hits.is_empty() {
        return Ok(hits.into_iter().collect());
    }
    match bare.split_once(':') {
        Some((m, _)) if !m.is_empty() => mod_part(m),
        _ => Err(format!("It refers to {bare}, which is not loaded.")),
    }
}

/// Why an export refuses a name that refers to an adjusted copy.
const COPY_NAMED: &str = "It refers to an adjusted copy, which a compatibility patch cannot name.";

/// The override entry for fix `f`, or why it cannot be one. `copies` are the profile's
/// adjusted copy ids.
fn part_of(
    raw: &RawLoad,
    copies: &HashSet<String>,
    fps: &mut Fingerprints,
    f: &ProfileFix,
) -> Result<Part, String> {
    if f.paused {
        return Err("It is paused. Resume it first.".to_string());
    }
    let (r, kind) = match &f.change {
        FixChange::AdjustedCopy { .. } => {
            return Err("An adjusted copy cannot be saved as a compatibility patch.".to_string())
        }
        FixChange::ModOrder { .. } => {
            return Err("A load order change cannot be saved as a compatibility patch.".to_string())
        }
        FixChange::AssetChoice { .. } => {
            return Err("An asset choice cannot be saved as a compatibility patch.".to_string())
        }
        FixChange::Disable { patch } => (patch.clone(), BindKind::Disable),
        FixChange::Priority { patch, .. } => (patch.clone(), BindKind::Priority),
        FixChange::Anchor { patch, .. } => (patch.clone(), BindKind::Anchor),
        FixChange::ApplyAs { patch, .. } => (patch.clone(), BindKind::ApplyAs),
        FixChange::SkipHunk { patch, .. } => (patch.clone(), BindKind::Hunk),
        FixChange::StackRule { patch, .. } => (patch.clone(), BindKind::Stack),
        FixChange::Order { rule, refs } => {
            let r = refs
                .iter()
                .find(|r| r.id == rule.patch)
                .cloned()
                .unwrap_or_else(|| fixes::pin(Some(raw), &rule.patch, true));
            (r, BindKind::Order)
        }
    };
    let ix = bound_one(raw, fps, &r, kind)?;
    let (target, patch) = compat_name(raw, ix)?;
    let fingerprint = fps.get(ix).map(|fp| PatchFingerprintLite {
        anchor: fp.anchor.clone(),
        payload: fp.payload.clone(),
        source: Some(fp.source.clone()),
        ordinal: Some(fp.ordinal),
    });
    let mut entry = Override {
        patch,
        fingerprint,
        ..Override::default()
    };
    let mut refs = vec![(target.clone(), Why::Changes)];
    match &f.change {
        FixChange::Disable { .. } => entry.disable = true,
        FixChange::Priority { priority, .. } => entry.priority = Some(*priority),
        FixChange::Order { rule, .. } => {
            for n in rule.before.iter().chain(&rule.after) {
                refs.extend(
                    name_mods(raw, copies, n)?
                        .into_iter()
                        .map(|m| (m, Why::Orders)),
                );
            }
            entry.before = rule.before.clone();
            entry.after = rule.after.clone();
        }
        FixChange::Anchor {
            anchors,
            sites,
            replace_author_anchors,
            ..
        } => {
            for w in anchors.iter().filter_map(|a| a.within.as_deref()) {
                refs.extend(
                    name_mods(raw, copies, w)?
                        .into_iter()
                        .map(|m| (m, Why::PlacesNext)),
                );
            }
            entry.anchors = anchors.iter().cloned().map(Lenient::Ok).collect();
            entry.sites = sites.iter().cloned().map(Lenient::Ok).collect();
            entry.replace_anchors = *replace_author_anchors;
        }
        FixChange::ApplyAs { mode, .. } => entry.apply_as = Some(*mode),
        FixChange::SkipHunk { hunk, .. } => {
            // The key must still name one of the patch's edits (D49).
            plan::check_hunk_key(raw, ix, hunk, fps)
                .map_err(|why| format!("Review this fix first: {why}."))?;
            if let Some(k) = hunk.keep.as_deref() {
                refs.extend(
                    name_mods(raw, copies, k)?
                        .into_iter()
                        .map(|m| (m, Why::KeepsChange)),
                );
            }
            entry.skip_hunks = vec![Lenient::Ok(hunk.clone())];
        }
        FixChange::StackRule {
            site,
            side,
            below,
            above,
            ..
        } => {
            // The other patch, as it binds now, named the way overrides name patches.
            let other = |r: &Option<PatchRef>, fps: &mut Fingerprints| -> Result<_, String> {
                r.as_ref()
                    .map(|r| {
                        bound_one(raw, fps, r, BindKind::Stack).and_then(|o| compat_name(raw, o))
                    })
                    .transpose()
            };
            let below = other(below, fps)?;
            let above = other(above, fps)?;
            for (m, _) in below.iter().chain(&above) {
                refs.push((m.clone(), Why::Stacks));
            }
            entry.stack_below = below.map(|(_, n)| n);
            entry.stack_above = above.map(|(_, n)| n);
            entry.stack_site = Some(site.clone());
            entry.stack_side = Some(*side);
        }
        FixChange::AdjustedCopy { .. }
        | FixChange::ModOrder { .. }
        | FixChange::AssetChoice { .. } => {}
    }
    Ok(Part {
        fix: f.clone(),
        entry,
        target,
        refs,
    })
}

/// The mod of raw patch `ix` and the patch as an override names it ("mod_id:patch_id").
fn compat_name(raw: &RawLoad, ix: usize) -> Result<(String, String), String> {
    let rp = raw
        .patches
        .get(ix)
        .ok_or("The patch it changes is not loaded.")?;
    let target = raw
        .mods
        .get(rp.mod_ix as usize)
        .map(|m| m.id.clone())
        .ok_or("The patch it changes is not loaded.")?;
    let id = rp.patch.id().ok_or("The patch it changes has no id.")?;
    let patch = if id
        .to_lowercase()
        .strip_prefix(&target)
        .is_some_and(|rest| rest.starts_with(':'))
    {
        id.to_string()
    } else {
        format!("{target}:{id}")
    };
    Ok((target, patch))
}

/// The one raw patch `r` applies to now, or why there is none.
fn bound_one(
    raw: &RawLoad,
    fps: &mut Fingerprints,
    r: &PatchRef,
    kind: BindKind,
) -> Result<usize, String> {
    let b = plan::bind(r, kind, fps);
    match (&b.status, b.targets.as_slice()) {
        // The profile has this anchor fix off until it is reviewed (plan.rs, §7.2).
        (BindingStatus::Drifted { anchor: true, .. }, _) if kind == BindKind::Anchor => {
            Err("Review this fix first.".to_string())
        }
        (
            BindingStatus::Bound | BindingStatus::Rebound { .. } | BindingStatus::Drifted { .. },
            [one],
        ) => Ok(*one),
        (BindingStatus::Waiting { mods }, _) => {
            let mods: Vec<String> = mods
                .iter()
                .map(|m| {
                    meta_of(raw, m)
                        .and_then(|x| x.name.clone())
                        .unwrap_or(m.clone())
                })
                .collect();
            Err(format!("Turn on {} first.", mods.join(", ")))
        }
        (BindingStatus::Ambiguous { .. }, _) | (_, [_, _, ..]) => {
            Err("Several patches match it. Pick the one it is for first.".to_string())
        }
        _ => Err("The patch it changes is not loaded.".to_string()),
    }
}

/// `compat_<a>_<b>`, sanitized to `^[a-z0-9_]{3,48}$`, with `_2`, `_3`, ... when taken.
fn unique_id(a: &str, b: Option<&str>, taken: &HashSet<String>) -> String {
    let raw = match b {
        Some(b) => format!("compat_{a}_{b}"),
        None => format!("compat_{a}"),
    };
    let mut base: String = raw
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else {
                '_'
            }
        })
        .collect();
    while base.contains("__") {
        base = base.replace("__", "_");
    }
    let fit = |s: &str, room: usize| -> String {
        let mut s: String = s.chars().take(room).collect();
        while s.ends_with('_') {
            s.pop();
        }
        s
    };
    let first = fit(&base, MAX_ID);
    if !taken.contains(&first) {
        return first;
    }
    let mut n = 2u32;
    loop {
        let suffix = format!("_{n}");
        let id = format!("{}{suffix}", fit(&base, MAX_ID - suffix.len()));
        if !taken.contains(&id) {
            return id;
        }
        n += 1;
    }
}

/// `s` with control characters made spaces and runs of space made one.
fn one_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn clip(s: &str) -> String {
    s.chars()
        .take(MAX_NAME)
        .collect::<String>()
        .trim()
        .to_string()
}

/// The user's name for the mod: one line, no path characters, no leading dots, at most
/// `MAX_NAME` characters. None when nothing is left.
fn clean_name(s: &str) -> Option<String> {
    let kept: String = s
        .chars()
        .filter(|c| !matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect();
    let name = clip(one_line(&kept).trim_start_matches(['.', ' ']));
    (!name.is_empty()).then_some(name)
}

/// The QuarterMaster features the qm.toml of `parts` uses (D61), in `QM_FEATURES` order, so
/// a QuarterMaster without one skips the mod instead of running part of it.
fn features_for(parts: &[Part], raw: &RawLoad) -> Vec<String> {
    let mut used: Vec<&str> = Vec::new();
    if !parts.is_empty() {
        used.push("overrides");
    }
    let anchors = |o: &Override| !o.anchors.is_empty() || !o.sites.is_empty() || o.replace_anchors;
    if parts.iter().any(|x| anchors(&x.entry)) {
        used.push("anchors");
    }
    // Rungs on a regex patch need a QuarterMaster that reads them (D52).
    let on_regex = |o: &Override| {
        raw.patches
            .iter()
            .any(|r| matches!(r.patch, Patch::Regex(_)) && name_matches(&o.patch, &r.patch))
    };
    if parts
        .iter()
        .any(|x| anchors(&x.entry) && on_regex(&x.entry))
    {
        used.push("regex_anchors");
    }
    if parts.iter().any(|x| x.entry.apply_as.is_some()) {
        used.push("apply_as");
    }
    if parts.iter().any(|x| !x.entry.skip_hunks.is_empty()) {
        used.push("skip_hunks");
    }
    if parts
        .iter()
        .any(|x| x.entry.stack_below.is_some() || x.entry.stack_above.is_some())
    {
        used.push("stack_rules");
    }
    qm_core::QM_FEATURES
        .iter()
        .filter(|f| used.contains(f))
        .map(|f| f.to_string())
        .collect()
}

fn manifest_for(
    id: &str,
    name: &str,
    requires: &[CompatRequirement],
    features: &[String],
) -> ModManifest {
    let mut m = ModManifest::new(id.to_string(), name.to_string(), EXPORT_VERSION.to_string());
    m.description = "A compatibility patch QuarterMaster made from fixes in a profile.".to_string();
    m.category = Some("Other".to_string());
    m.tags = vec!["compatibility".to_string()];
    m.qm_features = features.to_vec();
    for r in requires {
        m.relationships
            .requires
            .insert(r.mod_id.clone(), r.range.clone());
        m.relationships.load_after.push(r.mod_id.clone());
    }
    m
}

/// mod.json as JSON with `requires` in a stable order.
fn manifest_json(m: &ModManifest) -> QmResult<serde_json::Value> {
    let mut v = serde_json::to_value(m)
        .map_err(|e| internal("Could not write mod.json.", e.to_string()))?;
    let sorted: std::collections::BTreeMap<&String, &String> =
        m.relationships.requires.iter().collect();
    if let Some(rel) = v.get_mut("relationships").and_then(|r| r.as_object_mut()) {
        rel.insert(
            "requires".to_string(),
            serde_json::to_value(sorted)
                .map_err(|e| internal("Could not write mod.json.", e.to_string()))?,
        );
    }
    Ok(v)
}

fn toml_for(name: &str, profile: &str, parts: &[Part]) -> QmResult<String> {
    let mut out = format!(
        "# {}\n# A compatibility patch QuarterMaster made from fixes in the profile \"{}\".\n\
         # Each [[overrides]] entry is one fix. QuarterMaster applies an entry only while this\n\
         # mod requires every mod the entry refers to.\n\n\
         [manifest]\nversion = \"{EXPORT_VERSION}\"\n\n",
        one_line(name),
        one_line(profile)
    );
    let entries: Vec<(String, Override)> = parts
        .iter()
        .map(|x| (one_line(&x.fix.label), x.entry.clone()))
        .collect();
    let body = overrides_toml(&entries).map_err(|e| internal("Could not write qm.toml.", e))?;
    out.push_str(body.trim_end());
    out.push('\n');
    Ok(out)
}

fn readme_for(name: &str, profile: &str, parts: &[Part], requires: &[CompatRequirement]) -> String {
    let mut out = format!(
        "{}\n\nA compatibility patch QuarterMaster made from fixes in the profile \"{}\".\n\nIt changes:\n",
        one_line(name),
        one_line(profile)
    );
    for x in parts {
        let label = one_line(&x.fix.label);
        if label.contains(&x.entry.patch) {
            out.push_str(&format!("- {label}\n"));
        } else {
            out.push_str(&format!("- {label} ({})\n", x.entry.patch));
        }
    }
    out.push_str("\nIt needs:\n");
    for r in requires {
        let version = match &r.version {
            Some(v) if r.range != "*" => format!(" {v} or newer"),
            _ => String::new(),
        };
        out.push_str(&format!(
            "- {}{version}: {}\n",
            r.name,
            r.reasons.join("; ")
        ));
    }
    out.push_str("\nTurn it off in QuarterMaster to stop using it.\n");
    out
}

fn write_files(dir: &std::path::Path, files: &[CompatFile]) -> QmResult<()> {
    let io = |e: std::io::Error| {
        QmError::new(
            ErrorKind::Io,
            "The compatibility patch could not be written.",
        )
        .with_details(e.to_string())
    };
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(io)?;
    }
    std::fs::create_dir_all(dir).map_err(io)?;
    for f in files {
        std::fs::write(dir.join(&f.name), &f.text).map_err(io)?;
    }
    Ok(())
}

/// The files whose game code differs between two simulations: the patched text of every
/// file either touches (vanilla where one does not), and the module list.
fn differing_files(
    now: &SimulationReport,
    next: &SimulationReport,
    vanilla: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    let files: BTreeSet<&String> = now.traces.keys().chain(next.traces.keys()).collect();
    let text = |r: &SimulationReport, f: &str| -> Option<String> {
        r.traces
            .get(f)
            .map(|t| t.text.clone())
            .or_else(|| vanilla.get(f).cloned())
    };
    let mut out: Vec<String> = files
        .into_iter()
        .filter(|f| text(now, f) != text(next, f))
        .cloned()
        .collect();
    if now.module_registrations != next.module_registrations {
        out.push("modules".to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_sanitized_and_unique() {
        let none = HashSet::new();
        assert_eq!(
            unique_id("tactical_overhaul", Some("gcore"), &none),
            "compat_tactical_overhaul_gcore"
        );
        assert_eq!(unique_id("..\\Evil", Some("a-b"), &none), "compat_evil_a_b");
        let taken: HashSet<String> = ["compat_a_b", "compat_a_b_2"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(unique_id("a", Some("b"), &taken), "compat_a_b_3");
        let long = "x".repeat(80);
        let id = unique_id(&long, None, &none);
        assert_eq!(id.len(), MAX_ID);
        let taken: HashSet<String> = std::iter::once(id).collect();
        let id = unique_id(&long, None, &taken);
        assert!(id.len() <= MAX_ID && id.ends_with("_2"), "{id}");
    }

    #[test]
    fn names_lose_path_characters() {
        assert_eq!(clean_name("..\\..\\Evil/name").as_deref(), Some("Evilname"));
        assert_eq!(clean_name("  My\npatch\t ").as_deref(), Some("My patch"));
        assert_eq!(clean_name("../.."), None);
        assert_eq!(
            clean_name(&"n".repeat(100)).map(|s| s.len()),
            Some(MAX_NAME)
        );
    }
}
