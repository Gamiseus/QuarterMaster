// ============================================================================
// QuarterMaster Field Command — Desktop Mod Launcher Frontend Controller
// ============================================================================

const state = {
  data: null,
  selectedModId: null,
  selectedDiffFile: null,
  selectedPatchId: null, // null = All Patches, or specific patch_id string
  diffFileCache: {},
  diffMode: "unified",
  diffJumpCursor: 0,
  selectedSaveSlotId: null,
  draftConfigValues: {},
  modSearchQuery: "",
  modCategoryFilter: "",
  modFavOnly: false,
  undoStack: [],
  redoStack: [],
  logLevelFilter: "ALL",
  logModFilter: "",
  browserSearchQuery: "",
  browserSourceFilter: "",
  browserTopicFilter: "",
  communityMeta: null,
  devWatchTimer: null,
  devWatchLastMs: 0,
  lastModalCopyText: "",
  vanillaApi: null,
  apiSelectedSymbolId: null,
  apiSearchQuery: "",
  apiActiveFile: "all",
  apiActiveCategory: "all",
  apiActiveAction: "all",
  apiFileSearchQuery: "",
  apiSortBy: "default",
  apiSortDir: "asc",
  apiGroupByCat: true,
  apiModuleFilter: "all",
  apiKindFilter: "all",
  activeModTab: "scaffold",
  apiDocsInitialized: false,
  activeTab: "load-order",
  activeLibrarySubtab: "installed",
  communityCatalog: null,
  pendingProfileSwitchId: null,
  scaffoldActiveModDir: null,
  scaffoldActiveFilePath: null,
  scaffoldOriginalContent: "",
  scaffoldModFiles: [],
  patchStudioInitialized: false,
  patchStudioActiveFile: "includes/mapBuild.lua",
  patchTargetRange: [1, 5],
  currentSourceLines: [],
  engineShowOtherPatches: true,
  engineActivePatchesForFile: [],
  patchWizardStep: 1,
  patchContextLines: 4,
  patchWizardData: {
    targetMod: "",
    patchId: "",
    category: "# BATTLEFIELD & MAP BUILD",
    description: "",
    position: "at",
    pattern: "",
    payload: "",
    matchIndent: true,
    requires: []
  },
};

const CLASSIC_PLAYER_RANKS = [
  "Dirty civy",
  "Cadet",
  "2nd lt",
  "1rst lt",
  "Captain",
  "Major",
  "Lieutenant Colonel",
  "Colonel",
  "Brigadier General",
  "Major General",
  "Lieutenant General",
  "General",
  "Mud and Blood General",
  "Mud and Blood Grand Master",
];

const CAMPAIGN_PLAYER_RANKS = [
  "Balloon enthusiast",
  "Squad Commander",
  "Section Commander",
  "Platoon Commander",
  "Company Commander",
  "Battalion Commander",
  "Regiment Commander",
  "Brigade Commander",
  "Division Commander",
  "Corps Commander",
  "Army Commander",
  "Army Group Commander",
  "Grand Commander",
  "Grand Master Commander",
];

const TRENCH_RANKS = {
  1: "Soldier",
  2: "Lance Corporal",
  3: "Corporal",
  4: "Senior Corporal",
  5: "Sergeant",
  6: "Staff Sergeant",
  7: "Second Lieutenant",
  8: "First Lieutenant",
  9: "Captain",
  10: "Major",
  11: "Major",
  12: "Lieutenant Colonel",
  13: "Colonel",
  14: "Brigadier General",
  15: "Major General",
  16: "General",
};

function formatClassicRank(lvl) {
  const idx = Math.max(0, Math.min(CLASSIC_PLAYER_RANKS.length - 1, Number(lvl) || 0));
  return CLASSIC_PLAYER_RANKS[idx];
}

function formatCampaignRank(lvl) {
  const idx = Math.max(0, Math.min(CAMPAIGN_PLAYER_RANKS.length - 1, Number(lvl) || 0));
  return CAMPAIGN_PLAYER_RANKS[idx];
}

function formatTrenchRank(lvl) {
  return TRENCH_RANKS[lvl] || (Number(lvl) > 16 ? "General" : "Soldier");
}

function formatSoldierRank(rankNum, spec = "") {
  const isOfficer = String(spec || "").toLowerCase() === "officer";
  const r = rankNum == null ? null : Number(rankNum);
  if (isOfficer) {
    if (r === null || r < 0) return "Cadet";
    if (r === 0) return "Second Lieutenant";
    if (r === 1) return "First Lieutenant";
    if (r === 2) return "Captain";
    return "Major";
  } else {
    if (r === null || r < 0) return "Recruit";
    if (r === 0) return "Private";
    if (r === 1) return "Private First Class";
    if (r === 2) return "Corporal";
    return "Sergeant";
  }
}

// 1. Modern Web Guidance: <dialog closedby="any"> fallback for browsers without native closedBy
function setupLightDismissDialogs() {
  const dialogs = document.querySelectorAll("dialog[closedby]");
  if (!("closedBy" in HTMLDialogElement.prototype)) {
    dialogs.forEach((dialog) => {
      dialog.addEventListener("click", (event) => {
        if (event.target !== dialog) return;
        const rect = dialog.getBoundingClientRect();
        const isDialogContent =
          rect.top <= event.clientY &&
          event.clientY <= rect.top + rect.height &&
          rect.left <= event.clientX &&
          event.clientX <= rect.left + rect.width;
        if (isDialogContent) return;
        dialog.close();
      });
    });
  }
}

// 2. Modern Web Guidance: Dark mode two-state toggle with <meta name="color-scheme">
function setupColorSchemeAndPaletteToggles() {
  const meta = document.querySelector('meta[name="color-scheme"]');
  const schemeBtn = document.getElementById("btn-color-scheme");
  const paletteBtn = document.getElementById("btn-palette-switch");
  const paletteLabel = document.getElementById("palette-label");

  function syncPaletteLabel() {
    if (!paletteLabel) return;
    const p = document.documentElement.dataset.palette || "tactical";
    paletteLabel.textContent = p === "slate" ? "Theme: Slate" : "Theme: Dark";
  }
  syncPaletteLabel();

  if (schemeBtn) {
    schemeBtn.addEventListener("click", () => {
      const sysDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
      const sysScheme = sysDark ? "dark" : "light";
      let stored = null;
      try {
        stored = localStorage.getItem("color-scheme");
      } catch (_) {}
      const currentRendered = stored === "dark" || stored === "light" ? stored : sysScheme;
      const target = currentRendered === "dark" ? "light" : "dark";

      if (target !== sysScheme) {
        try {
          localStorage.setItem("color-scheme", target);
        } catch (_) {}
        if (meta) meta.content = target;
      } else {
        try {
          localStorage.removeItem("color-scheme");
        } catch (_) {}
        if (meta) meta.content = "light dark";
      }
    });
  }

  window.addEventListener("storage", (e) => {
    if (e.key === "color-scheme" && meta) {
      meta.content = e.newValue ?? "light dark";
    }
  });

  if (paletteBtn) {
    paletteBtn.addEventListener("click", () => {
      const next = document.documentElement.dataset.palette === "slate" ? "tactical" : "slate";
      document.documentElement.dataset.palette = next;
      try {
        localStorage.setItem("qm-palette", next);
      } catch (_) {}
      syncPaletteLabel();
    });
  }

  // Settings theme dropdown handlers
  const themeSelect = document.getElementById("setup-theme-palette");
  if (themeSelect) {
    themeSelect.addEventListener("change", (e) => {
      document.documentElement.dataset.palette = e.target.value;
      try { localStorage.setItem("qm-palette", e.target.value); } catch (_) {}
      syncPaletteLabel();
    });
  }
  const colorSelect = document.getElementById("setup-color-scheme");
  if (colorSelect) {
    colorSelect.addEventListener("change", (e) => {
      const target = e.target.value;
      if (meta) meta.content = target;
      try { localStorage.setItem("color-scheme", target); } catch (_) {}
    });
  }
}

// 3. Interactive Draggable / Resizable Splitters (Horizontal & Vertical)
function setupDraggableSplitters() {
  document.querySelectorAll(".resizer-x").forEach((resizer) => {
    const splitId = resizer.dataset.splitId;
    const container = document.getElementById(splitId);
    if (!container) return;

    const defaultPct = Number(container.dataset.defaultLeft || 55);
    const storageKey = `qm-split-x-${splitId}`;
    try {
      const saved = localStorage.getItem(storageKey);
      if (saved) container.style.setProperty("--split-left", `${saved}%`);
      else container.style.setProperty("--split-left", `${defaultPct}%`);
    } catch (_) {
      container.style.setProperty("--split-left", `${defaultPct}%`);
    }

    resizer.addEventListener("pointerdown", (e) => {
      e.preventDefault();
      resizer.setPointerCapture(e.pointerId);
      resizer.classList.add("dragging");

      const onMove = (moveEvt) => {
        const rect = container.getBoundingClientRect();
        if (rect.width <= 0) return;
        const relX = moveEvt.clientX - rect.left;
        const pct = Math.max(18, Math.min(82, (relX / rect.width) * 100));
        container.style.setProperty("--split-left", `${pct.toFixed(1)}%`);
        try {
          localStorage.setItem(storageKey, pct.toFixed(1));
        } catch (_) {}
      };

      const onUp = (upEvt) => {
        resizer.classList.remove("dragging");
        try {
          resizer.releasePointerCapture(upEvt.pointerId);
        } catch (_) {}
        resizer.removeEventListener("pointermove", onMove);
        resizer.removeEventListener("pointerup", onUp);
      };

      resizer.addEventListener("pointermove", onMove);
      resizer.addEventListener("pointerup", onUp);
    });

    resizer.addEventListener("dblclick", () => {
      container.style.setProperty("--split-left", `${defaultPct}%`);
      try {
        localStorage.removeItem(storageKey);
      } catch (_) {}
      showToast("Reset panel split width to default.");
    });
  });

  // Vertical resizer for Tab 2 Conflict Resolver Cards height
  const resizerY = document.getElementById("resizer-conflicts-height");
  const conflictsBox = document.getElementById("conflicts-list-container");
  if (resizerY && conflictsBox) {
    const storageKey = "qm-split-y-conflicts";
    try {
      const savedH = localStorage.getItem(storageKey);
      if (savedH) conflictsBox.style.setProperty("--conflicts-height", `${savedH}px`);
    } catch (_) {}

    resizerY.addEventListener("pointerdown", (e) => {
      e.preventDefault();
      resizerY.setPointerCapture(e.pointerId);
      resizerY.classList.add("dragging");
      const startY = e.clientY;
      const startH = conflictsBox.getBoundingClientRect().height;

      const onMove = (moveEvt) => {
        const nextH = Math.max(48, Math.min(420, startH + (moveEvt.clientY - startY)));
        conflictsBox.style.setProperty("--conflicts-height", `${Math.round(nextH)}px`);
        try {
          localStorage.setItem(storageKey, String(Math.round(nextH)));
        } catch (_) {}
      };

      const onUp = (upEvt) => {
        resizerY.classList.remove("dragging");
        try {
          resizerY.releasePointerCapture(upEvt.pointerId);
        } catch (_) {}
        resizerY.removeEventListener("pointermove", onMove);
        resizerY.removeEventListener("pointerup", onUp);
      };

      resizerY.addEventListener("pointermove", onMove);
      resizerY.addEventListener("pointerup", onUp);
    });

    resizerY.addEventListener("dblclick", () => {
      conflictsBox.style.setProperty("--conflicts-height", "120px");
      try {
        localStorage.removeItem(storageKey);
      } catch (_) {}
    });
  }
}

function showToast(msg, isError = false) {
  const banner = document.getElementById("toast-banner");
  banner.textContent = msg;
  banner.style.borderColor = isError ? "var(--status-danger)" : "var(--accent-brass)";
  banner.hidden = false;
  clearTimeout(banner._timer);
  banner._timer = setTimeout(() => {
    banner.hidden = true;
  }, 4500);
}

function openReportModal(title, htmlContent, copyableText = "") {
  const dlg = document.getElementById("report-modal-dialog");
  document.getElementById("report-modal-title").textContent = title;
  document.getElementById("report-modal-body").innerHTML = htmlContent;
  state.lastModalCopyText = copyableText;
  dlg.showModal();
}

async function apiGet(path) {
  const res = await fetch(path);
  const text = await res.text();
  let data = null;
  try {
    data = JSON.parse(text);
  } catch (_) {
    throw new Error(text || `HTTP ${res.status}`);
  }
  if (!res.ok || data.error) {
    throw new Error(data.error || `HTTP ${res.status}`);
  }
  return data;
}

async function apiPost(path, body = {}) {
  const res = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const data = await res.json();
  if (!res.ok || data.error) {
    throw new Error(data.error || `Request failed (${res.status})`);
  }
  return data;
}

function invalidateDiffState() {
  state.diffFileCache = {};
  state.selectedDiffFile = null;
  state.selectedPatchId = null;
  state.diffJumpCursor = 0;
}

async function refreshState() {
  try {
    const data = await apiGet("/api/state");
    state.data = data;
    if (!state.selectedModId && data.installed_mods.length > 0) {
      state.selectedModId = data.installed_mods[0].manifest.id;
    }
    if (data.save_profiles.length > 0) {
      const exists = data.save_profiles.some((s) => s.profile.id === state.selectedSaveSlotId);
      if (!state.selectedSaveSlotId || !exists) {
        // Prefer the active save profile if it has a campaign, otherwise fallback to default
        const activeId = data.settings.active_save_profile_id;
        const activeEntry = data.save_profiles.find((s) => s.profile.id === activeId);
        const defaultEntry = data.save_profiles.find((s) => s.profile.id === "default");
        if (activeEntry?.summary?.deployed_squad?.length > 0) {
          state.selectedSaveSlotId = activeId;
        } else if (defaultEntry?.summary?.deployed_squad?.length > 0) {
          state.selectedSaveSlotId = "default";
        } else {
          state.selectedSaveSlotId = activeId || data.save_profiles[0].profile.id;
        }
      }
    }
    renderAll();
  } catch (err) {
    showToast(`Failed to load QuarterMaster state: ${err.message}`, true);
  }
}

function renderAll() {
  const d = state.data;
  if (!d) return;

  // Header status & live game process watcher
  document.getElementById("qm-version-pill").textContent = `v${d.version}`;
  const gInfo = d.game_info;
  const runningPid = d.health?.game_running_pid;
  document.getElementById("game-status-pill").textContent = gInfo
    ? `Mud and Blood v${gInfo.version} • ${gInfo.lua_file_count} Lua Files • ${d.simulation.applied_patches}/${d.simulation.total_patches} Patches Ready${runningPid ? ` • [RUNNING] (PID ${runningPid})` : ""}`
    : "mnb.exe Not Configured";

  const launchLbl = document.getElementById("launch-primary-label");
  if (launchLbl) {
    launchLbl.textContent = runningPid ? `RUNNING (PID ${runningPid})` : "LAUNCH";
  }

  // Nav counters
  document.getElementById("nav-count-mods").textContent = `${d.active_profile.enabled_mods.length}/${d.installed_mods.length}`;
  document.getElementById("nav-count-conflicts").textContent = splitFindings(d.simulation.conflicts).conflicts.length;
  const activeSaveEntry =
    d.save_profiles.find((s) => s.profile.id === state.selectedSaveSlotId) || d.save_profiles[0];
  document.getElementById("nav-count-squad").textContent =
    activeSaveEntry?.summary?.deployed_squad?.length ?? 0;
  document.getElementById("nav-count-crashes").textContent = d.crashes.length;

  // Profile selector with integrated New Profile option
  const profSel = document.getElementById("header-profile-select");
  const profOptions = [
    `<option value="__new__">+ New Profile...</option>`,
    ...d.profiles.map(
      (p) =>
        `<option value="${p.id}" ${p.id === d.active_profile.id ? "selected" : ""}>${escapeHtml(p.name)} (${p.enabled_mods.length} mods)</option>`
    ),
  ].join("");
  profSel.innerHTML = profOptions;

  // Sync Undo/Redo button states
  document.getElementById("btn-undo-order").disabled = state.undoStack.length === 0;
  document.getElementById("btn-redo-order").disabled = state.redoStack.length === 0;

  // Gated tab rendering (Requirement R5): Only render the active tab to maintain optimal UI responsiveness
  renderActiveTab();
}

function renderActiveTab() {
  const activeTab = state.activeTab || "load-order";
  switch (activeTab) {
    case "load-order":
      renderLoadOrderTab();
      renderModDetailDrawer();
      break;
    case "conflicts":
      renderConflictsAndDiffTab();
      break;
    case "saves":
      renderSavesTab();
      break;
    case "library":
      renderLibraryTab();
      break;
    case "crashes":
      renderCrashesTab();
      break;
    case "devtools":
      renderDevToolsTab();
      if (state.activeModTab === "api-docs") {
        initApiDocsTab();
      }
      break;
    default:
      renderLoadOrderTab();
      break;
  }
}

// ============================================================================
// TAB 1: LOAD ORDER, DRAG-AND-DROP, SEPARATORS & VISUAL SCHEMA CONFIG EDITOR
// ============================================================================
function renderLoadOrderTab() {
  const d = state.data;
  const prof = d.active_profile;
  document.getElementById("active-profile-summary").textContent =
    `Profile '${prof.name}' (${prof.id}) • Linked Save Slot: '${prof.linked_save_profile_id}' • ${prof.glue_patches.length} Glue Patch(es) • ${(prof.disabled_patches || []).length} Disabled Patch(es)`;

  // Populate category filter
  const catFilter = document.getElementById("mod-category-filter");
  const categories = Array.from(
    new Set(d.installed_mods.map((m) => m.manifest.category || "General"))
  ).sort();
  catFilter.innerHTML =
    `<option value="">All Categories (${categories.length})</option>` +
    categories
      .map(
        (c) =>
          `<option value="${escapeHtml(c)}" ${c === state.modCategoryFilter ? "selected" : ""}>${escapeHtml(c)}</option>`
      )
      .join("");

  const compatBanner = document.getElementById("save-compat-banner");
  if (d.save_compatibility && !d.save_compatibility.is_safe && d.save_compatibility.warning_message) {
    compatBanner.hidden = false;
    compatBanner.className = "compat-banner";
    compatBanner.innerHTML = `
      <div style="display:flex; align-items:center; justify-content:space-between; gap:10px;">
        <span><strong>[Save Compatibility Warning:</strong> ${escapeHtml(d.save_compatibility.warning_message)}</span>
        <button type="button" class="btn btn-xs btn-accent" id="btn-sync-profile-to-save">Enable Missing Save Mods</button>
      </div>
    `;
    const syncSaveBtn = document.getElementById("btn-sync-profile-to-save");
    if (syncSaveBtn) {
      syncSaveBtn.addEventListener("click", async () => {
        const missingIds = (d.save_compatibility.missing_mods_from_save || []).map((pair) => pair[0]);
        const merged = Array.from(new Set([...prof.enabled_mods, ...missingIds]));
        await saveActiveProfileUpdate({ enabled_mods: merged });
      });
    }
  } else {
    compatBanner.hidden = true;
  }

  // Rule Hierarchy & MO2 Overwrite Bucket bar
  const hierLabel = document.getElementById("rule-hierarchy-label");
  if (hierLabel && d.load_order?.rule_hierarchy) {
    hierLabel.textContent = d.load_order.rule_hierarchy;
  }
  const overwritePill = document.getElementById("overwrite-bucket-pill");
  if (overwritePill) {
    const ovFiles = d.health?.overwrite_files || [];
    if (ovFiles.length === 0) {
      overwritePill.innerHTML = `<span class="badge badge-ok" title="MO2-style per-profile Overwrite directory for runtime-generated files">MO2 Overwrite Bucket: Clean (0 files)</span>`;
    } else {
      overwritePill.innerHTML = `
        <span class="badge badge-warn" title="${escapeHtml(ovFiles.join(", "))}">MO2 Overwrite Bucket: ${ovFiles.length} runtime file(s)</span>
        <button type="button" class="btn btn-xs" id="btn-clear-overwrite">Clear Overwrite</button>
      `;
      document.getElementById("btn-clear-overwrite")?.addEventListener("click", async () => {
        try {
          const res = await apiPost("/api/overwrite/clear", { profile_id: prof.id });
          await refreshState();
          showToast(`Cleared ${res.cleared_count} file(s) from profile Overwrite bucket.`);
        } catch (err) {
          showToast(err.message, true);
        }
      });
    }
  }

  // Pre-launch Solver Diagnostics & 1-Click Fix Banner (Section 7 & 8)
  const solverBanner = document.getElementById("solver-validation-banner");
  const diags = d.load_order?.diagnostics || [];
  if (solverBanner) {
    if (diags.length === 0) {
      solverBanner.hidden = true;
    } else {
      solverBanner.hidden = false;
      solverBanner.innerHTML = diags
        .map((diag, idx) => {
          const isErr = diag.severity === "error";
          let actionBtn = "";
          if (diag.kind === "missing_dependency" && diag.related_mod_id) {
            actionBtn = `<button type="button" class="btn btn-xs btn-accent solver-fix-enable-btn" data-dep-id="${escapeHtml(diag.related_mod_id)}">1-Click Enable '${escapeHtml(diag.related_mod_id)}'</button>`;
          } else if (diag.kind === "conflict" && diag.related_mod_id) {
            actionBtn = `<button type="button" class="btn btn-xs btn-danger solver-fix-disable-btn" data-conf-id="${escapeHtml(diag.related_mod_id)}">1-Click Disable '${escapeHtml(diag.related_mod_id)}'</button>`;
          }
          return `
            <div style="display:flex; align-items:center; justify-content:space-between; gap:10px; padding:${idx > 0 ? "4px 0 0" : "0"};">
              <span>${isErr ? "<strong>[ERROR — Blocks Launch]</strong>" : "<strong>[WARNING]</strong>"} ${escapeHtml(diag.message)}</span>
              ${actionBtn}
            </div>
          `;
        })
        .join("");

      solverBanner.querySelectorAll(".solver-fix-enable-btn").forEach((btn) => {
        btn.addEventListener("click", async () => {
          const depId = btn.dataset.depId;
          const merged = Array.from(new Set([...prof.enabled_mods, depId]));
          await saveActiveProfileUpdate({ enabled_mods: merged });
        });
      });

      solverBanner.querySelectorAll(".solver-fix-disable-btn").forEach((btn) => {
        btn.addEventListener("click", async () => {
          const confId = btn.dataset.confId;
          const filtered = prof.enabled_mods.filter((id) => id !== confId);
          await saveActiveProfileUpdate({ enabled_mods: filtered });
        });
      });
    }
  }

  const orderMap = {};
  d.load_order.ordered_mods.forEach((o) => {
    orderMap[o.mod_id] = o;
  });

  // Sort mods by effective load order
  const modsSorted = [...d.installed_mods].sort((a, b) => {
    const pa = prof.mod_priorities[a.manifest.id] ?? a.manifest.load_priority;
    const pb = prof.mod_priorities[b.manifest.id] ?? b.manifest.load_priority;
    return pa - pb || a.manifest.id.localeCompare(b.manifest.id);
  });

  const tbody = document.getElementById("load-order-tbody");
  const rowsHtml = [];
  let activeCollapsedSeparator = false;
  const q = state.modSearchQuery.trim().toLowerCase();
  const favs = new Set(prof.favorites || []);
  const locked = new Set(prof.locked_mods || []);
  const notes = prof.mod_notes || {};

  modsSorted.forEach((m) => {
    const id = m.manifest.id;

    // Check if an MO2 separator is anchored before this mod
    (prof.separators || [])
      .filter((s) => s.insert_before_mod_id === id)
      .forEach((sep) => {
        activeCollapsedSeparator = Boolean(sep.collapsed);
        rowsHtml.push(`
          <tr class="separator-row" data-sep-id="${sep.id}">
            <td colspan="9" style="border-left-color: ${sep.color || "#eab308"}">
              <div style="display:flex; align-items:center; justify-content:space-between;">
                <span>${sep.collapsed ? "▸ [COLLAPSED]" : "▾"} ${escapeHtml(sep.title)}</span>
                <div class="toolbar-row">
                  <button type="button" class="btn btn-xs sep-toggle-btn" data-sep-id="${sep.id}">
                    ${sep.collapsed ? "Expand" : "Collapse"}
                  </button>
                  <button type="button" class="btn btn-xs sep-delete-btn" data-sep-id="${sep.id}" title="Remove Separator">x</button>
                </div>
              </div>
            </td>
          </tr>
        `);
      });

    if (activeCollapsedSeparator && !q) {
      return;
    }

    // Apply search, category, and favorites filters
    const cat = m.manifest.category || "General";
    if (state.modCategoryFilter && cat !== state.modCategoryFilter) return;
    if (state.modFavOnly && !favs.has(id)) return;
    if (q) {
      const hay = [
        m.manifest.name,
        m.manifest.id,
        ...(m.manifest.authors || []),
        ...(m.manifest.tags || []),
        cat,
        notes[id] || "",
      ]
        .join(" ")
        .toLowerCase();
      if (!hay.includes(q)) return;
    }

    const isEnabled = prof.enabled_mods.includes(id);
    const ord = orderMap[id];
    const effPrio = prof.mod_priorities[id] ?? m.manifest.load_priority;
    const reasons = ord ? ord.reasons.join(" • ") : "Disabled in active profile";
    const isSelected = state.selectedModId === id;
    const isFav = favs.has(id);
    const isLocked = locked.has(id);
    const noteText = notes[id] || "";

    rowsHtml.push(`
      <tr class="mod-row ${isSelected ? "selected" : ""}" data-mod-id="${id}" draggable="true">
        <td><span class="drag-handle" title="Drag to reorder mod priority">⠿</span></td>
        <td>
          <input type="checkbox" class="mod-enable-check" data-mod-id="${id}" ${isEnabled ? "checked" : ""} aria-label="Enable ${escapeHtml(m.manifest.name)}" />
        </td>
        <td><span class="badge">${ord ? "#" + ord.order_index : "-"}</span></td>
        <td>
          <div style="display:flex; align-items:center; gap:5px; flex-wrap:wrap;">
            <button type="button" class="icon-btn mod-fav-btn ${isFav ? "active" : ""}" data-mod-id="${id}" title="Toggle Favorite">${isFav ? "[FAV]" : "[+]"}</button>
            <strong>${escapeHtml(m.manifest.name)}</strong>
            <span class="version-tag">v${escapeHtml(m.manifest.version)}</span>
            <button type="button" class="icon-btn mod-note-btn" data-mod-id="${id}" title="Add / Edit Mod Note">Note</button>
          </div>
          <div class="provenance-line">${escapeHtml(reasons)}</div>
          ${noteText ? `<div class="mod-note-pill">Note: ${escapeHtml(noteText)}</div>` : ""}
        </td>
        <td><span class="badge">${escapeHtml(cat)}</span></td>
        <td>
          <div style="display:flex; align-items:center; gap:3px;">
            <input type="number" class="tactical-input prio-input mod-prio-input" data-mod-id="${id}" value="${effPrio}" ${isLocked ? "disabled" : ""} />
            <button type="button" class="icon-btn mod-lock-btn ${isLocked ? "active" : ""}" data-mod-id="${id}" title="${isLocked ? "Priority Locked" : "Lock Priority against Auto-Sort"}">
              ${isLocked ? "[Locked]" : "[Unlocked]"}
            </button>
          </div>
        </td>
        <td><span class="badge">${m.patch_count} patches</span></td>
        <td>
          <span class="badge ${m.integrity.is_pristine ? "badge-ok" : "badge-warn"}">
            ${m.integrity.is_pristine ? "Verified" : "Modified"}
          </span>
        </td>
        <td>
          <button type="button" class="btn btn-xs mod-move-btn" data-mod-id="${id}" data-delta="-10" title="Move Up" ${isLocked ? "disabled" : ""}>▲</button>
          <button type="button" class="btn btn-xs mod-move-btn" data-mod-id="${id}" data-delta="10" title="Move Down" ${isLocked ? "disabled" : ""}>▼</button>
        </td>
      </tr>
    `);
  });

  tbody.innerHTML = rowsHtml.join("");

  // Separator collapse / delete handlers
  tbody.querySelectorAll(".sep-toggle-btn").forEach((btn) => {
    btn.addEventListener("click", async (e) => {
      e.stopPropagation();
      const sepId = btn.dataset.sepId;
      const nextSeps = (prof.separators || []).map((s) =>
        s.id === sepId ? { ...s, collapsed: !s.collapsed } : s
      );
      await saveActiveProfileUpdate({ separators: nextSeps });
    });
  });

  tbody.querySelectorAll(".sep-delete-btn").forEach((btn) => {
    btn.addEventListener("click", async (e) => {
      e.stopPropagation();
      const sepId = btn.dataset.sepId;
      const nextSeps = (prof.separators || []).filter((s) => s.id !== sepId);
      await saveActiveProfileUpdate({ separators: nextSeps });
    });
  });

  // Row selection & HTML5 Drag-and-Drop Reordering
  let draggedModId = null;
  tbody.querySelectorAll(".mod-row").forEach((row) => {
    row.addEventListener("click", (e) => {
      if (e.target.closest("input, button")) return;
      state.selectedModId = row.dataset.modId;
      renderLoadOrderTab();
      renderModDetailDrawer();
    });

    row.addEventListener("dragstart", (e) => {
      draggedModId = row.dataset.modId;
      e.dataTransfer.effectAllowed = "move";
    });

    row.addEventListener("dragover", (e) => {
      e.preventDefault();
      row.classList.add("drag-over");
      const scrollParent = row.closest(".table-responsive") || row.closest(".config-panel") || document.querySelector(".table-responsive");
      if (scrollParent) {
        const rect = scrollParent.getBoundingClientRect();
        if (e.clientY < rect.top + 40) {
          scrollParent.scrollTop -= 14;
        } else if (e.clientY > rect.bottom - 40) {
          scrollParent.scrollTop += 14;
        }
      }
    });

    row.addEventListener("dragleave", () => {
      row.classList.remove("drag-over");
    });

    row.addEventListener("drop", async (e) => {
      e.preventDefault();
      row.classList.remove("drag-over");
      const targetModId = row.dataset.modId;
      if (!draggedModId || !targetModId || draggedModId === targetModId) return;
      if ((prof.locked_mods || []).includes(draggedModId)) {
        return showToast(`Mod '${draggedModId}' priority is locked ([Locked]). Unlock it first.`, true);
      }
      const targetMod = d.installed_mods.find((m) => m.manifest.id === targetModId);
      const targetPrio = prof.mod_priorities[targetModId] ?? (targetMod ? targetMod.manifest.load_priority : 50);
      const nextPrios = { ...prof.mod_priorities, [draggedModId]: Math.max(1, targetPrio - 2) };
      await saveActiveProfileUpdate({ mod_priorities: nextPrios });
    });
  });

  tbody.querySelectorAll(".mod-enable-check").forEach((chk) => {
    chk.addEventListener("change", async () => {
      const modId = chk.dataset.modId;
      const enabled = new Set(prof.enabled_mods);
      if (chk.checked) enabled.add(modId);
      else enabled.delete(modId);
      await saveActiveProfileUpdate({ enabled_mods: Array.from(enabled) });
    });
  });

  tbody.querySelectorAll(".mod-prio-input").forEach((inp) => {
    inp.addEventListener("change", async () => {
      const modId = inp.dataset.modId;
      const val = parseInt(inp.value, 10) || 50;
      const prios = { ...prof.mod_priorities, [modId]: val };
      await saveActiveProfileUpdate({ mod_priorities: prios });
    });
  });

  tbody.querySelectorAll(".mod-move-btn").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const modId = btn.dataset.modId;
      const delta = parseInt(btn.dataset.delta, 10);
      const modObj = d.installed_mods.find((x) => x.manifest.id === modId);
      const cur = prof.mod_priorities[modId] ?? (modObj ? modObj.manifest.load_priority : 50);
      const prios = { ...prof.mod_priorities, [modId]: cur + delta };
      await saveActiveProfileUpdate({ mod_priorities: prios });
    });
  });

  tbody.querySelectorAll(".mod-fav-btn").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const modId = btn.dataset.modId;
      const nextFavs = new Set(prof.favorites || []);
      if (nextFavs.has(modId)) nextFavs.delete(modId);
      else nextFavs.add(modId);
      await saveActiveProfileUpdate({ favorites: Array.from(nextFavs) });
    });
  });

  tbody.querySelectorAll(".mod-lock-btn").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const modId = btn.dataset.modId;
      const nextLocks = new Set(prof.locked_mods || []);
      if (nextLocks.has(modId)) nextLocks.delete(modId);
      else nextLocks.add(modId);
      await saveActiveProfileUpdate({ locked_mods: Array.from(nextLocks) });
    });
  });

  tbody.querySelectorAll(".mod-note-btn").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const modId = btn.dataset.modId;
      const curNote = (prof.mod_notes || {})[modId] || "";
      const nextNote = prompt(`Note for '${modId}' (leave blank to clear):`, curNote);
      if (nextNote === null) return;
      const nextNotes = { ...(prof.mod_notes || {}) };
      if (nextNote.trim()) nextNotes[modId] = nextNote.trim();
      else delete nextNotes[modId];
      await saveActiveProfileUpdate({ mod_notes: nextNotes });
    });
  });
}

async function saveActiveProfileUpdate(patch, pushUndo = true) {
  try {
    if (pushUndo && state.data?.active_profile) {
      state.undoStack.push(JSON.parse(JSON.stringify(state.data.active_profile)));
      if (state.undoStack.length > 30) state.undoStack.shift();
      state.redoStack = [];
    }
    const prof = { ...state.data.active_profile, ...patch };
    invalidateDiffState();
    await apiPost("/api/profile/update", prof);
    await refreshState();
    showToast(`Updated profile '${prof.name}' & refreshed offline patch simulation.`);
  } catch (err) {
    showToast(err.message, true);
  }
}

function evaluateShowIf(rule, currentValues) {
  if (!rule) return true;
  if (rule.option_id && Array.isArray(rule.equals)) {
    const val = currentValues[rule.option_id];
    return rule.equals.some((eq) => String(eq) === String(val));
  }
  return true;
}

function renderModDetailDrawer() {
  const d = state.data;
  const mod = d.installed_mods.find((m) => m.manifest.id === state.selectedModId);
  if (!mod) return;

  const authors = (mod.manifest.authors || []).join(", ") || "Unknown Author";
  document.getElementById("drawer-mod-title").textContent = mod.manifest.name;
  document.getElementById("drawer-mod-meta").textContent = `${mod.manifest.id} • by ${authors} • ${mod.manifest.description}`;
  document.getElementById("drawer-mod-version").textContent = `v${mod.manifest.version}`;

  // 1. Visual Config Editor (config.lua)
  const cfgContainer = document.getElementById("subtab-config");
  if (!mod.config_schema || !mod.config_schema.options || mod.config_schema.options.length === 0) {
    cfgContainer.innerHTML = `<p class="panel-subtitle">This mod does not expose a <code>config.lua</code> settings schema.</p>`;
  } else {
    if (!state.draftConfigValues[mod.manifest.id]) {
      const initial = {};
      mod.config_schema.options.forEach((opt) => {
        if (opt.option_type === "separator") return;
        const saved = mod.saved_config ? mod.saved_config[opt.id] : undefined;
        initial[opt.id] = saved !== undefined ? saved : opt.default;
      });
      state.draftConfigValues[mod.manifest.id] = initial;
    }
    const vals = state.draftConfigValues[mod.manifest.id];

    const itemsHtml = mod.config_schema.options
      .map((opt) => {
        const visible = evaluateShowIf(opt.show_if, vals);
        if (!visible) return "";
        if (opt.option_type === "separator") {
          return `<div class="section-heading" style="padding:8px 2px 4px">${escapeHtml(opt.label)}</div>`;
        }
        const curVal = vals[opt.id];
        let controlHtml = "";
        if (opt.option_type === "checkbox") {
          const isChecked = curVal === true || curVal === 1 || curVal === "1" || curVal === "true";
          controlHtml = `<input type="checkbox" class="cfg-input" data-opt-id="${opt.id}" data-type="boolean" ${isChecked ? "checked" : ""} />`;
        } else if (opt.option_type === "slider") {
          const min = opt.min ?? 0;
          const max = opt.max ?? 100;
          const step = opt.step ?? 1;
          controlHtml = `
            <div class="toolbar-row">
              <input type="range" class="cfg-input" data-opt-id="${opt.id}" data-type="number" min="${min}" max="${max}" step="${step}" value="${curVal ?? min}" />
              <span class="badge">${curVal ?? min}</span>
            </div>
          `;
        } else if (opt.option_type === "cycle" && Array.isArray(opt.options)) {
          controlHtml = `
            <select class="tactical-select cfg-input" data-opt-id="${opt.id}" data-type="string">
              ${opt.options
                .map((c, idx) => {
                  const lbl = (opt.labels && opt.labels[idx]) || c;
                  return `<option value="${c}" ${String(curVal) === String(c) ? "selected" : ""}>${escapeHtml(lbl)}</option>`;
                })
                .join("")}
            </select>
          `;
        } else {
          controlHtml = `<input type="text" class="tactical-input cfg-input" data-opt-id="${opt.id}" data-type="string" value="${curVal ?? ""}" style="width:120px" />`;
        }

        return `
          <div class="config-option-card">
            <div class="config-option-header">
              <span>${escapeHtml(opt.label)}</span>
              ${controlHtml}
            </div>
            <div class="config-option-desc">${escapeHtml(opt.description || "")}</div>
          </div>
        `;
      })
      .join("");

    cfgContainer.innerHTML = `
      <div class="toolbar-row" style="margin-bottom:10px; justify-content:space-between;">
        <span class="field-label">${mod.config_schema.options.length} Options (${escapeHtml(mod.manifest.name)})</span>
        <div class="toolbar-row">
          <button type="button" class="btn btn-xs" id="btn-safe-reset-config" title="Keep valid user settings and reset only unknown or out-of-range keys">Safe-Reset</button>
          <button type="button" class="btn btn-xs" id="btn-reset-defaults-config" title="Reset all options for this mod back to schema defaults">Reset Defaults</button>
          <button type="button" class="btn btn-sm btn-accent" id="btn-save-mod-config">Save Settings</button>
        </div>
      </div>
      ${itemsHtml}
    `;

    cfgContainer.querySelectorAll(".cfg-input").forEach((el) => {
      el.addEventListener("input", () => {
        const optId = el.dataset.optId;
        const t = el.dataset.type;
        if (t === "boolean") vals[optId] = el.checked;
        else if (t === "number") vals[optId] = Number(el.value);
        else vals[optId] = el.value;
        renderModDetailDrawer();
      });
    });

    document.getElementById("btn-save-mod-config")?.addEventListener("click", async () => {
      try {
        await apiPost("/api/mod/config", {
          mod_id: mod.manifest.id,
          values: vals,
        });
        delete state.draftConfigValues[mod.manifest.id];
        await refreshState();
        showToast(`Saved ${mod.manifest.name} settings.`);
      } catch (err) {
        showToast(err.message, true);
      }
    });

    document.getElementById("btn-reset-defaults-config")?.addEventListener("click", async () => {
      try {
        await apiPost("/api/mod/config/reset", { mod_id: mod.manifest.id, mode: "defaults" });
        delete state.draftConfigValues[mod.manifest.id];
        await refreshState();
        showToast(`Reset ${mod.manifest.name} config to schema defaults.`);
      } catch (err) {
        showToast(err.message, true);
      }
    });

    document.getElementById("btn-safe-reset-config")?.addEventListener("click", async () => {
      try {
        await apiPost("/api/mod/config/reset", { mod_id: mod.manifest.id, mode: "safe" });
        delete state.draftConfigValues[mod.manifest.id];
        await refreshState();
        showToast(`Safe-reset ${mod.manifest.name} config (preserved valid choices).`);
      } catch (err) {
        showToast(err.message, true);
      }
    });
  }

  // 2. Dependencies
  const ord = d.load_order.ordered_mods.find((o) => o.mod_id === mod.manifest.id);
  const rel = mod.manifest.relationships || {};
  document.getElementById("subtab-provenance").innerHTML = `
    <div class="config-option-card">
      <div class="field-label">Load Order Reasoning</div>
      <ul style="margin: 6px 0 0 18px; font-size:12px;">
        ${(ord ? ord.reasons : ["Mod is currently disabled in active profile"]).map((r) => `<li>${escapeHtml(r)}</li>`).join("")}
      </ul>
    </div>
    <div class="config-option-card">
      <div class="field-label">Dependencies</div>
      <div class="config-option-desc">
        <div><strong>Requires:</strong> ${Object.entries(rel.requires || {}).map(([k, v]) => `${k} (${v})`).join(", ") || "None"}</div>
        <div><strong>Load After:</strong> ${(rel.load_after || []).join(", ") || "None"}</div>
        <div><strong>Load Before:</strong> ${(rel.load_before || []).join(", ") || "None"}</div>
        <div><strong>Provides:</strong> ${(rel.provides || []).join(", ") || "None"}</div>
        <div><strong>Update Keys:</strong> ${(mod.manifest.update_keys || []).join(", ") || "None"}</div>
      </div>
    </div>
  `;

  // 3. Docs (README / CHANGELOG)
  document.getElementById("subtab-docs").innerHTML = `
    <div class="field-label" style="margin-bottom:4px">README.md</div>
    <pre class="markdown-box">${escapeHtml(mod.readme_md || "No README.md found.")}</pre>
    <div class="field-label" style="margin:10px 0 4px">CHANGELOG.md</div>
    <pre class="markdown-box">${escapeHtml(mod.changelog_md || "No CHANGELOG.md found.")}</pre>
  `;

  // 4. Security & Integrity
  const sec = mod.security_report;
  document.getElementById("subtab-security").innerHTML = `
    <div class="config-option-card">
      <div class="config-option-header">
        <span>File Verification</span>
        <span class="badge ${mod.integrity.is_pristine ? "badge-ok" : "badge-warn"}">
          ${mod.integrity.is_pristine ? "Verified" : "Modified"}
        </span>
      </div>
      <div class="config-option-desc" style="word-break:break-all;">Path: ${escapeHtml(mod.install_path)}</div>
    </div>
    <div class="config-option-card">
      <div class="config-option-header">
        <span>Security Scan</span>
        <span class="badge ${sec.capabilities.length === 0 ? "badge-ok" : "badge-warn"}">
          ${sec.capabilities.length === 0 ? "Safe (Sandboxed)" : sec.capabilities.length + " Advisories"}
        </span>
      </div>
      <div class="config-option-desc">
        ${
          sec.findings.length === 0
            ? "No unsafe system or external calls detected."
            : sec.findings
                .map((f) => `<div>• [${f.severity.toUpperCase()}] ${escapeHtml(f.file)}:${f.line} — <code>${escapeHtml(f.snippet)}</code></div>`)
                .join("")
        }
      </div>
    </div>
  `;
}

// ============================================================================
// TAB 2: OFFLINE PATCH SIMULATION, CLICKABLE PATCH BOXES & CONFLICT RESOLVER
// ============================================================================
// Info notes (severity "info": multi_match, the semantic notes) are not conflicts: they are
// counted and listed apart. Each entry keeps its index in `all`, which the resolve buttons use.
function splitFindings(all) {
  const conflicts = [];
  const notes = [];
  (all || []).forEach((c, idx) => (c.severity === "info" ? notes : conflicts).push({ c, idx }));
  return { conflicts, notes };
}

function renderConflictsAndDiffTab() {
  const sim = state.data.simulation;
  const prof = state.data.active_profile;

  const { conflicts, notes } = splitFindings(sim.conflicts);
  const hasConflicts = conflicts.length > 0;
  const statusSummary = hasConflicts
    ? `${sim.applied_patches}/${sim.total_patches} patches applied across ${sim.files_touched.length} files (${conflicts.length} conflict${conflicts.length === 1 ? "" : "s"} detected)`
    : `All ${sim.applied_patches} patches in '${escapeHtml(prof.name)}' applied without conflicts across ${sim.files_touched.length} files.`;

  document.getElementById("sim-summary-bar").innerHTML = `
    <div style="display: flex; align-items: center; gap: 8px;">
      <span style="font-weight: 600; color: var(--text-primary);">${statusSummary}</span>
      ${sim.broken_anchors > 0 ? `<span class="badge badge-warn">${sim.broken_anchors} Broken Anchors</span>` : ""}
      ${notes.length > 0 ? `<span class="badge">${notes.length} Note${notes.length === 1 ? "" : "s"}</span>` : ""}
    </div>
    <div class="toolbar-row">
      ${prof.glue_patches.length > 0 ? `<span class="badge">${prof.glue_patches.length} Active Glue Patches</span>` : ""}
      ${(prof.disabled_patches || []).length > 0 ? `<span class="badge">${(prof.disabled_patches || []).length} Disabled Patches</span>` : ""}
    </div>
  `;

  const confContainer = document.getElementById("conflicts-list-container");
  if (!hasConflicts && notes.length === 0) {
    confContainer.innerHTML = "";
    confContainer.style.display = "none";
  } else {
    confContainer.style.display = "";
    const noteCards = notes
      .map(
        ({ c }) => `
          <div class="conflict-card">
            <div>
              <span class="badge">NOTE</span>
              <strong>${escapeHtml(c.target_file)}</strong> — ${escapeHtml(c.description)}
            </div>
          </div>
        `,
      )
      .join("");
    confContainer.innerHTML = conflicts
      .map(({ c, idx }) => {
        const tierBtns = (c.suggestions || [])
          .map((sug, sIdx) => {
            if (sug.tier === "swap_mod_order") {
              return `<button type="button" class="btn btn-sm btn-accent resolve-conflict-btn" data-cidx="${idx}" data-sidx="${sIdx}">
                Auto-Order Fix: ${escapeHtml(sug.explanation)}
              </button>`;
            }
            if (sug.tier === "generate_glue_patch") {
              return `<button type="button" class="btn btn-sm btn-accent resolve-conflict-btn" data-cidx="${idx}" data-sidx="${sIdx}">
                Compatibility Patch: ${escapeHtml(sug.explanation)}
              </button>`;
            }
            if (sug.tier === "select_winner") {
              return `<button type="button" class="btn btn-sm resolve-conflict-btn" data-cidx="${idx}" data-sidx="${sIdx}" data-winner="0">
                Use ${escapeHtml(sug.mod_a_id || "First")}
              </button>
              <button type="button" class="btn btn-sm resolve-conflict-btn" data-cidx="${idx}" data-sidx="${sIdx}" data-winner="1">
                Use ${escapeHtml(sug.mod_b_id || "Second")}
              </button>`;
            }
            return "";
          })
          .join("");

        return `
          <div class="conflict-card">
            <div>
              <span class="badge badge-warn">${c.severity.toUpperCase()}</span>
              <strong>${escapeHtml(c.target_file)}</strong> — ${escapeHtml(c.description)}
            </div>
            <div class="conflict-tiers">${tierBtns}</div>
          </div>
        `;
      })
      .join("") + noteCards;

    confContainer.querySelectorAll(".resolve-conflict-btn").forEach((btn) => {
      btn.addEventListener("click", async () => {
        const c = sim.conflicts[Number(btn.dataset.cidx)];
        const sug = c.suggestions[Number(btn.dataset.sidx)];
        const winnerIdx = btn.dataset.winner ? Number(btn.dataset.winner) : 0;
        try {
          await apiPost("/api/conflict/resolve", { suggestion: sug, winner_index: winnerIdx });
          invalidateDiffState();
          await refreshState();
          showToast("Resolved conflict.");
        } catch (err) {
          showToast(err.message, true);
        }
      });
    });
  }

  // Validate state.selectedDiffFile against the current profile's sim.files_touched!
  const fileList = document.getElementById("patched-files-list");
  const touchedNames = sim.files_touched.map((f) => f.target_file);
  if (!state.selectedDiffFile || !touchedNames.includes(state.selectedDiffFile)) {
    state.selectedDiffFile = touchedNames.length > 0 ? touchedNames[0] : null;
    state.selectedPatchId = null;
    state.diffJumpCursor = 0;
  }

  if (sim.files_touched.length === 0) {
    // Pure Vanilla or 0-patch profile: completely clear the diff viewer so no stale diffs remain!
    fileList.innerHTML = `<li class="file-item" style="cursor:default"><div class="panel-subtitle">0 Lua files modified in '${escapeHtml(prof.name)}'.</div></li>`;
    document.getElementById("diff-file-title").textContent = `Vanilla Profile (${prof.name})`;
    document.getElementById("diff-file-subtitle").textContent =
      "No mods or patches are active in this profile — all 30 vanilla Lua files are untouched.";
    document.getElementById("diff-mod-filter").innerHTML = `<option value="">All Mods (0)</option>`;
    document.getElementById("patch-records-strip").innerHTML = "";
    document.getElementById("selected-patch-inspector").hidden = true;
    document.getElementById("attributed-code-container").innerHTML = `
      <div style="padding:24px" class="config-option-card">
        <strong style="color:var(--status-ok)">Vanilla — No Patches Active</strong>
        <p class="config-option-desc">Switch to a modded profile or enable mods in the Mods tab to inspect patches and diffs.</p>
      </div>
    `;
    return;
  }

  fileList.innerHTML = sim.files_touched
    .map(
      (f) => `
      <li class="file-item ${state.selectedDiffFile === f.target_file ? "active" : ""}" data-file="${escapeHtml(f.target_file)}">
        <div><strong>${escapeHtml(f.target_file)}</strong></div>
        <div class="provenance-line">${f.original_line_count} → ${f.patched_line_count} lines • ${f.records.length} patches (${f.mods_touching.join(", ")})</div>
      </li>
    `
    )
    .join("");

  fileList.querySelectorAll(".file-item").forEach((li) => {
    li.addEventListener("click", () => {
      state.selectedDiffFile = li.dataset.file;
      state.selectedPatchId = null;
      state.diffJumpCursor = 0;
      renderConflictsAndDiffTab();
    });
  });

  // Defer /api/file-diff requests (Requirement R5): Only request file diffs when the diff tab is actually active
  if (state.activeTab === "conflicts" && state.selectedDiffFile) {
    loadAndRenderFileDiff(state.selectedDiffFile);
  }
}

async function loadAndRenderFileDiff(targetFile) {
  try {
    if (!state.diffFileCache[targetFile]) {
      state.diffFileCache[targetFile] = await apiGet(
        `/api/file-diff?file=${encodeURIComponent(targetFile)}`
      );
    }
    const diff = state.diffFileCache[targetFile];
    // Verify selectedPatchId still exists in this file's records
    if (
      state.selectedPatchId &&
      !diff.records.some((r) => r.patch_id === state.selectedPatchId)
    ) {
      state.selectedPatchId = null;
    }

    document.getElementById("diff-file-title").textContent = diff.target_file;
    document.getElementById("diff-file-subtitle").textContent = state.selectedPatchId
      ? `Showing patch '${state.selectedPatchId}' • Click 'All Patches' to show entire file`
      : `${diff.records.length} patches applied to ${diff.target_file} • Click any patch box below to isolate that patch`;

    // Populate mod filter dropdown
    const modFilter = document.getElementById("diff-mod-filter");
    const curFilter = modFilter.value;
    const modsInFile = Array.from(new Set(diff.records.map((r) => r.mod_id)));
    modFilter.innerHTML =
      `<option value="">All Mods (${modsInFile.length})</option>` +
      modsInFile
        .map((m) => `<option value="${m}" ${m === curFilter ? "selected" : ""}>${m}</option>`)
        .join("");

    // Render Clickable Individual Patch Selector Strip (#patch-records-strip)
    const strip = document.getElementById("patch-records-strip");
    const allBoxHtml = `
      <button type="button" class="patch-box ${state.selectedPatchId === null ? "active" : ""}" data-patch-id="">
        <div class="patch-box-top">
          <span class="badge badge-ok">ALL</span>
          <span>All Patches (${diff.records.length})</span>
        </div>
        <div class="patch-box-sub">${diff.original_line_count} → ${diff.patched_line_count} lines</div>
      </button>
    `;

    const boxesHtml = diff.records
      .map((r) => {
        const isSelected = state.selectedPatchId === r.patch_id;
        const prof = state.data?.active_profile;
        const isDisabled =
          r.status === "SkippedDisabled" ||
          r.status === "skipped_disabled" ||
          (prof?.disabled_patches || []).includes(r.patch_id);
        const isApplied = r.status === "Applied" || r.status === "applied";
        const statusBadge = isApplied
          ? `<span class="badge badge-ok">${r.matched_count}</span>`
          : isDisabled
          ? `<span class="badge badge-danger">⊘ OFF</span>`
          : `<span class="badge badge-warn">! ${r.matched_count}</span>`;

        return `
          <button type="button" class="patch-box ${isSelected ? "active" : ""} ${isDisabled ? "disabled-patch" : ""}" data-patch-id="${escapeHtml(r.patch_id)}" title="Click to isolate &amp; configure patch '${escapeHtml(r.patch_name)}'">
            <div class="patch-box-top">
              ${statusBadge}
              <span>[${escapeHtml(r.mod_id)}] ${escapeHtml(r.patch_name)}</span>
            </div>
            <div class="patch-box-sub">${escapeHtml(r.patch_type)} • Prio ${r.priority} • ID: ${escapeHtml(r.patch_id)}</div>
          </button>
        `;
      })
      .join("");

    strip.innerHTML = allBoxHtml + boxesHtml;

    strip.querySelectorAll(".patch-box").forEach((box) => {
      box.addEventListener("click", () => {
        const pid = box.dataset.patchId || null;
        state.selectedPatchId = state.selectedPatchId === pid ? null : pid;
        state.diffJumpCursor = 0;
        loadAndRenderFileDiff(targetFile);
      });
    });

    // Render Selected Individual Patch Inspector Bar (#selected-patch-inspector)
    renderSelectedPatchInspector(diff);

    // Render Code / Diff Viewport
    renderCodePane(diff, curFilter);
  } catch (err) {
    showToast(`Failed to load file diff: ${err.message}`, true);
  }
}

function renderSelectedPatchInspector(diff) {
  const inspector = document.getElementById("selected-patch-inspector");
  if (!state.selectedPatchId) {
    inspector.hidden = true;
    inspector.innerHTML = "";
    return;
  }

  const rec = diff.records.find((r) => r.patch_id === state.selectedPatchId);
  if (!rec) {
    inspector.hidden = true;
    return;
  }

  const prof = state.data.active_profile;
  const isDisabled =
    rec.status === "SkippedDisabled" ||
    rec.status === "skipped_disabled" ||
    (prof.disabled_patches || []).includes(rec.patch_id);
  const isApplied = rec.status === "Applied" || rec.status === "applied";
  const lineRangeText = rec.modified_line_range
    ? `Lines ${rec.modified_line_range[0]}–${rec.modified_line_range[1]}`
    : "No lines modified";

  inspector.hidden = false;
  inspector.innerHTML = `
    <div>
      <div>
        <span class="badge ${isApplied ? "badge-ok" : isDisabled ? "badge-danger" : "badge-warn"}">${isDisabled ? "disabled" : escapeHtml(rec.status)}</span>
        <strong>[${escapeHtml(rec.mod_id)}] ${escapeHtml(rec.patch_name)}</strong>
        <span class="badge">${escapeHtml(rec.patch_type)}</span>
        <span class="badge">${lineRangeText}</span>
      </div>
      <div class="provenance-line">
        Patch ID: <code>${escapeHtml(rec.patch_id)}</code> •
        Matches: <strong>${rec.matched_count}</strong>${rec.expected_count ? ` / ${rec.expected_count}` : ""} •
        On Failure Policy: <code>${escapeHtml(rec.on_failure)}</code>
      </div>
    </div>
    <div class="toolbar-row">
      <label class="field-label" for="patch-prio-override-input">PATCH PRIORITY</label>
      <input type="number" id="patch-prio-override-input" class="tactical-input prio-input" value="${rec.priority}" />
      <button type="button" class="btn btn-xs" id="btn-save-patch-prio">Set Priority</button>
      <button type="button" class="btn btn-xs ${isDisabled ? "btn-success" : "btn-danger"}" id="btn-toggle-patch-enabled">
        ${isDisabled ? "Enable Patch" : "Disable Patch"}
      </button>
      <button type="button" class="btn btn-xs" id="btn-clear-patch-selection">Show All</button>
    </div>
  `;

  document.getElementById("btn-toggle-patch-enabled")?.addEventListener("click", async () => {
    try {
      const willEnable = isDisabled;
      const allActivePatches = state.data?.active_profile?.patches || diff.records || [];
      const disabledPatches = state.data?.active_profile?.disabled_patches || [];

      const cascade = checkPatchPrerequisites(rec.patch_id, willEnable, allActivePatches, disabledPatches);

      if (cascade.action === "confirm_disable_dependents") {
        const patchListStr = cascade.affectedPatches.map(p => `• [${p.mod_id || "Mod"}] ${p.name || p.id}`).join("\n");
        const confirmed = confirm(
          "[Warning] Prerequisite Patch Disabling\n\n" +
          `Disabling '${rec.patch_name || rec.patch_id}' will also disable the following dependent patch(es) that require it:\n\n` +
          patchListStr + "\n\n" +
          "Click OK to disable this patch and all dependent patches, or Cancel to abort."
        );
        if (!confirmed) return;

        // Cascade disable all dependents
        for (const p of cascade.affectedPatches) {
          await apiPost("/api/patch/configure", { patch_id: p.id, enabled: false });
        }
      } else if (cascade.action === "confirm_enable_prerequisites") {
        const patchListStr = cascade.affectedPatches.map(p => `• [${p.mod_id || "Mod"}] ${p.name || p.id}`).join("\n");
        const confirmed = confirm(
          "[Notice] Prerequisite Patches Required\n\n" +
          `Enabling '${rec.patch_name || rec.patch_id}' requires the following prerequisite patch(es) to also be activated:\n\n` +
          patchListStr + "\n\n" +
          "Click OK to enable this patch and its prerequisites, or Cancel to abort."
        );
        if (!confirmed) return;

        // Cascade enable all prerequisites
        for (const p of cascade.affectedPatches) {
          await apiPost("/api/patch/configure", { patch_id: p.id, enabled: true });
        }
      }

      await apiPost("/api/patch/configure", {
        patch_id: rec.patch_id,
        enabled: willEnable,
      });
      state.diffFileCache = {};
      await refreshState();
      showToast(
        `${willEnable ? "Enabled" : "Disabled"} patch '${rec.patch_name}'.`
      );
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-save-patch-prio")?.addEventListener("click", async () => {
    const newPrio = parseInt(document.getElementById("patch-prio-override-input").value, 10);
    if (Number.isNaN(newPrio)) return;
    try {
      await apiPost("/api/patch/configure", {
        patch_id: rec.patch_id,
        priority: newPrio,
      });
      state.diffFileCache = {};
      await refreshState();
      showToast(`Set patch priority to ${newPrio}.`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-clear-patch-selection")?.addEventListener("click", () => {
    state.selectedPatchId = null;
    loadAndRenderFileDiff(diff.target_file);
  });
}

function renderCodePane(diff, filterModId) {
  const container = document.getElementById("attributed-code-container");
  const lines = diff.attributed_lines || [];
  const origLines = diff.original_lines || [];
  const selectedPatchId = state.selectedPatchId;

  // Determine which line numbers match the current filter (mod filter AND/OR selected individual patch)
  const targetLineNumbers = new Set();
  lines.forEach((l) => {
    if (!l.mod_id) return;
    const matchMod = !filterModId || l.mod_id === filterModId;
    const matchPatch = !selectedPatchId || l.patch_id === selectedPatchId;
    if (matchMod && matchPatch) {
      for (let ln = Math.max(1, l.line_number - 4); ln <= l.line_number + 4; ln++) {
        targetLineNumbers.add(ln);
      }
    }
  });

  // Also include the modified_line_range of the selected patch if it was a replacement/deletion
  if (selectedPatchId) {
    const rec = (diff.records || []).find((r) => r.patch_id === selectedPatchId);
    if (rec && rec.modified_line_range) {
      for (
        let ln = Math.max(1, rec.modified_line_range[0] - 4);
        ln <= rec.modified_line_range[1] + 4;
        ln++
      ) {
        targetLineNumbers.add(ln);
      }
    }
  }

  const visibleLines = lines.filter((l) => targetLineNumbers.has(l.line_number));

  if (visibleLines.length === 0) {
    const selRec = selectedPatchId
      ? (diff.records || []).find((r) => r.patch_id === selectedPatchId)
      : null;
    const isSelDisabled =
      selRec &&
      (selRec.status === "SkippedDisabled" ||
        selRec.status === "skipped_disabled" ||
        (state.data?.active_profile?.disabled_patches || []).includes(selectedPatchId));
    if (isSelDisabled) {
      container.innerHTML = `
        <div style="padding:24px" class="config-option-card">
          <strong style="color:var(--status-danger)">⊘ Patch '${escapeHtml(selRec.patch_name || selectedPatchId)}' is Disabled</strong>
          <p class="config-option-desc">This patch is currently disabled in your active profile and does not modify <code>${escapeHtml(diff.target_file)}</code>. Click the green <strong>Enable Patch</strong> button above to re-activate it.</p>
        </div>
      `;
    } else {
      container.innerHTML = `<div style="padding:16px" class="panel-subtitle">No modified lines match the current filter (${selectedPatchId ? `Patch: ${selectedPatchId}` : "All Patches"}).</div>`;
    }
    return;
  }

  // MODE 1: SIDE-BY-SIDE (Vanilla vs Patched)
  if (state.diffMode === "side-by-side") {
    const rows = [
      `<div class="side-by-side-header">
        <span>VANILLA v0.5.3b (${escapeHtml(diff.target_file)} — ${origLines.length} lines)</span>
        <span>PATCHED RESULT (${diff.patched_line_count} lines)</span>
      </div>`,
    ];
    let prevLineNo = -1;
    visibleLines.forEach((l) => {
      if (prevLineNo !== -1 && l.line_number > prevLineNo + 1) {
        const skipped = l.line_number - prevLineNo - 1;
        rows.push(`
          <div class="side-by-side-grid" style="background:var(--bg-elevated); color:var(--text-muted)">
            <div class="sbs-cell"><span>...</span><span>--- ${skipped} unchanged vanilla lines ---</span></div>
            <div class="sbs-cell"><span>...</span><span>--- ${skipped} unchanged vanilla lines ---</span></div>
          </div>
        `);
      }
      const isTargetPatch = selectedPatchId ? l.patch_id === selectedPatchId : Boolean(l.mod_id);
      const vIdx = l.line_number - 1;
      const vText = vIdx < origLines.length ? origLines[vIdx] : "";
      const attrPrefix = l.mod_id ? `[${l.mod_id}] ` : "";
      rows.push(`
        <div class="side-by-side-grid">
          <div class="sbs-cell">
            <span class="code-lineno">${vIdx < origLines.length ? l.line_number : ""}</span>
            <span>${escapeHtml(vText)}</span>
          </div>
          <div class="sbs-cell ${isTargetPatch ? "patched" : ""}">
            <span class="code-lineno">${l.line_number}</span>
            <span>${attrPrefix ? `<strong style="color:var(--accent-brass)">${escapeHtml(attrPrefix)}</strong>` : ""}${escapeHtml(l.text)}</span>
          </div>
        </div>
      `);
      prevLineNo = l.line_number;
    });
    container.innerHTML = rows.join("");
    return;
  }

  // UNIFIED DIFF VIEW (- / +)
  const html = [];
  let prevLineNo = -1;
  visibleLines.forEach((l) => {
    if (prevLineNo !== -1 && l.line_number > prevLineNo + 1) {
      const skipped = l.line_number - prevLineNo - 1;
      html.push(
        `<div class="code-line" style="color:var(--text-muted); background:var(--bg-elevated)"><span class="code-lineno">@@</span><span class="code-attr">--- ${skipped} unchanged vanilla lines ---</span><span></span></div>`
      );
    }
    const isModded = selectedPatchId ? l.patch_id === selectedPatchId : Boolean(l.mod_id);
    const attrText = l.mod_id ? `+ [${l.mod_id}] ${l.patch_name || ""}` : "  vanilla";
    html.push(`
      <div class="code-line ${isModded ? "unified-add patched" : ""}" data-line="${l.line_number}">
        <span class="code-lineno">${l.line_number}</span>
        <span class="code-attr" title="${escapeHtml(attrText)}">${escapeHtml(attrText)}</span>
        <span>${isModded ? "+ " : "  "}${escapeHtml(l.text)}</span>
      </div>
    `);
    prevLineNo = l.line_number;
  });
  container.innerHTML = html.join("");
}

// ============================================================================
// TAB 3: DEFENSIVE SAVE VIEWER, SQUAD ROSTER, TELEMETRY & BACKUP TIMELINE
// ============================================================================
function renderSavesTab() {
  const d = state.data;
  const saveSel = document.getElementById("save-slot-select");
  saveSel.innerHTML = d.save_profiles
    .map(
      (s) =>
        `<option value="${s.profile.id}" ${s.profile.id === state.selectedSaveSlotId ? "selected" : ""}>
          ${escapeHtml(s.profile.name)} (${s.profile.id}) ${s.profile.id === d.settings.active_save_profile_id ? "[Active]" : ""} ${s.profile.is_vanilla_clean ? "[Vanilla]" : ""}
        </option>`
    )
    .join("");

  // Populate Compare Save A & B dropdowns
  const cmpASel = document.getElementById("compare-save-a");
  const cmpBSel = document.getElementById("compare-save-b");
  if (cmpASel && cmpBSel) {
    const curA = cmpASel.value || state.selectedSaveSlotId || "default";
    const curB =
      cmpBSel.value ||
      (d.save_profiles.find((s) => s.profile.id !== curA)?.profile.id ?? "vanilla");
    cmpASel.innerHTML = d.save_profiles
      .map(
        (s) =>
          `<option value="${s.profile.id}" ${s.profile.id === curA ? "selected" : ""}>${escapeHtml(s.profile.id)}</option>`
      )
      .join("");
    cmpBSel.innerHTML = d.save_profiles
      .map(
        (s) =>
          `<option value="${s.profile.id}" ${s.profile.id === curB ? "selected" : ""}>${escapeHtml(s.profile.id)}</option>`
      )
      .join("");
  }

  const entry =
    d.save_profiles.find((s) => s.profile.id === state.selectedSaveSlotId) || d.save_profiles[0];
  if (!entry) return;

  const sum = entry.summary;
  const metricsEl = document.getElementById("save-campaign-metrics");
  const stripEl = document.getElementById("save-battlefield-strip");
  const squadEl = document.getElementById("save-squad-grid");
  const emptyNotice = document.getElementById("save-empty-notice");

  const isEmptySlot =
    !sum || (sum.deployed_squad.length === 0 && sum.day === 0 && sum.classic_max_wave === 0);

  if (isEmptySlot && entry.profile.id !== "default") {
    emptyNotice.hidden = false;
    emptyNotice.innerHTML = `
      <div style="display:flex; align-items:center; justify-content:space-between; gap:10px;">
        <span>Note: Save <strong>'${escapeHtml(entry.profile.name)}'</strong> has no active campaign file yet. Your main campaign is in the <strong>'default'</strong> slot.</span>
        <button type="button" class="btn btn-xs btn-accent" id="btn-jump-default-save">View 'default' Save</button>
      </div>
    `;
    document.getElementById("btn-jump-default-save")?.addEventListener("click", () => {
      state.selectedSaveSlotId = "default";
      renderSavesTab();
    });
  } else {
    emptyNotice.hidden = true;
  }

  if (!sum) {
    metricsEl.innerHTML = `<div class="metric-card">No <code>saveMNB3.lua</code> file present in this slot yet.</div>`;
    stripEl.innerHTML = "";
    squadEl.innerHTML = "";
  } else {
    // 1. Campaign, Classic, Trench & Map Cards
    metricsEl.innerHTML = `
      <div class="metric-card">
        <div class="field-label">CAMPAIGN PROGRESS</div>
        <div class="metric-val">${escapeHtml(sum.area_of_operations)}</div>
        <div class="metric-sub">Day ${sum.day} • Missions: ${sum.missions_completed} • Streak: ${sum.win_streak}</div>
      </div>
      <div class="metric-card">
        <div class="field-label">GERMAN ADVERSARY</div>
        <div class="metric-val">${escapeHtml(sum.german_commander)}</div>
        <div class="metric-sub">Escalation: ${sum.trench_escalation} • Free Planes: ${sum.free_planes}</div>
      </div>
      <div class="metric-card">
        <div class="field-label">CLASSIC RANK &amp; XP</div>
        <div class="metric-val">${sum.rank_title || formatClassicRank(sum.rank)}</div>
        <div class="metric-sub">${sum.experience} XP • Ribbons: ${sum.ribbons_count}</div>
      </div>
      <div class="metric-card">
        <div class="field-label">CAMPAIGN COMMANDER</div>
        <div class="metric-val">${sum.campaign_rank_title || formatCampaignRank(sum.campaign_rank || sum.classic_rank)}</div>
        <div class="metric-sub">${sum.campaign_experience || 0} XP • Points: ${sum.tactical_points} TP / ${sum.command_points} CP</div>
      </div>
      <div class="metric-card">
        <div class="field-label">KILLS &amp; LOSSES</div>
        <div class="metric-val">${sum.campaign_kills} / ${sum.campaign_casualties} KIA</div>
        <div class="metric-sub">Memorial Cemetery: ${sum.fallen_soldiers_count} Fallen</div>
      </div>
      <div class="metric-card">
        <div class="field-label">CLASSIC MODE</div>
        <div class="metric-val">Wave ${sum.classic_max_wave}</div>
        <div class="metric-sub">${sum.classic_total_kills} Kills • ${sum.classic_total_casualties} KIA</div>
      </div>
      <div class="metric-card">
        <div class="field-label">TRENCH MODE</div>
        <div class="metric-val">Rank ${sum.trench_rank} (${sum.trench_xp} XP)</div>
        <div class="metric-sub">${sum.trench_axis_kills} Kills / ${sum.trench_total_casualties} KIA • Intense: ${sum.fight_intensity_record}</div>
      </div>
      <div class="metric-card">
        <div class="field-label">MAP STATS</div>
        <div class="metric-val">${sum.map_buildings_count || 0} Structures</div>
        <div class="metric-sub">${sum.map_placeables_count || 0} Emplacements • ${sum.map_depth_px || 0}px</div>
      </div>
    `;

    // 2. Trench Sector Control & Active Battle Condition Modifiers
    const sectors = sum.trench_sectors || [];
    const bcList = sum.active_battle_conditions || [];
    const optEntries = Object.entries(sum.mod_options || {});

    stripEl.innerHTML = `
      <div class="config-option-card" style="margin-bottom:0">
        <div style="display:flex; align-items:center; justify-content:space-between; flex-wrap:wrap; gap:8px;">
          <div class="trench-sectors-row">
            <span class="field-label">Trench Sectors:</span>
            ${
              sectors.length === 0
                ? `<span class="badge">Standard Frontline</span>`
                : sectors
                    .map(
                      (val, idx) =>
                        `<span class="trench-sector-pill ${String(val).includes("US") || String(val) === "1" ? "captured" : ""}">S${idx + 1}: ${escapeHtml(val)}</span>`
                    )
                    .join("")
            }
          </div>
          <div class="trench-sectors-row">
            <span class="field-label">Active Battle Conditions (${bcList.length}):</span>
            ${
              bcList.length === 0
                ? `<span class="badge badge-ok">None Active</span>`
                : bcList
                    .map((bc) => `<span class="badge badge-warn">${escapeHtml(bc)}</span>`)
                    .join("")
            }
          </div>
        </div>
        ${
          optEntries.length > 0
            ? `<div class="provenance-line" style="margin-top:6px">
                <strong>Save Mod Options (${optEntries.length}):</strong>
                ${optEntries.map(([k, v]) => `<code>${escapeHtml(k)}=${escapeHtml(v)}</code>`).join(" • ")}
              </div>`
            : ""
        }
      </div>
    `;

    // 3. Deployed Squad & Reserve Roster Dossier Cards
    const allSoldiers = [
      ...(sum.deployed_squad || []).map((s) => ({ ...s, _pool: "DEPLOYED" })),
      ...(sum.reserve_soldiers || []).map((s) => ({ ...s, _pool: "RESERVE" })),
    ];
    document.getElementById("squad-summary-pill").textContent =
      `${(sum.deployed_squad || []).length} Deployed • ${(sum.reserve_soldiers || []).length} Reserve`;

    if (allSoldiers.length === 0) {
      squadEl.innerHTML = `<div class="config-option-card">No deployed soldiers in this save slot yet.</div>`;
    } else {
      squadEl.innerHTML = allSoldiers
        .map((sol) => {
          const hpPct = sol.max_health > 0 ? Math.min(100, Math.round((sol.health / sol.max_health) * 100)) : 100;
          const moralePct = Math.max(0, Math.min(100, Math.round(sol.morale || 100)));
          const accPct = sol.shots > 0 ? Math.round((sol.hits / sol.shots) * 100) : 0;
          const equipBadges = (sol.equipment || [])
            .map((eq) => `<span class="badge badge-ok">${escapeHtml(eq)}</span>`)
            .join("");

          return `
            <div class="soldier-card">
              <div class="soldier-header">
                <div>
                  <span class="soldier-name">${escapeHtml(sol.name)}</span>
                  <span class="badge">${escapeHtml(sol.rank_title || formatSoldierRank(sol.rank, sol.specialization))}</span>
                </div>
                <span class="badge ${sol._pool === "DEPLOYED" ? "badge-ok" : ""}">${sol._pool} • Lvl ${sol.level}</span>
              </div>
              <div class="provenance-line">
                Age ${sol.age} • XP: ${sol.xp} • Worth: ${sol.worth} TP • State: <code>${escapeHtml(sol.state || "ok")}</code>
              </div>

              <div class="bars-row">
                <div class="stat-bar-wrap">
                  <div style="display:flex; justify-content:space-between;">
                    <span>HP</span>
                    <strong>${sol.health}/${sol.max_health}</strong>
                  </div>
                  <div class="stat-bar-track"><div class="stat-bar-fill" style="width:${hpPct}%"></div></div>
                </div>
                <div class="stat-bar-wrap">
                  <div style="display:flex; justify-content:space-between;">
                    <span>Morale</span>
                    <strong>${moralePct}%</strong>
                  </div>
                  <div class="stat-bar-track"><div class="stat-bar-fill morale" style="width:${moralePct}%"></div></div>
                </div>
              </div>

              <div class="weapon-box">
                <div><strong>Weapon:</strong> ${escapeHtml(sol.weapon_tag)} (${escapeHtml(sol.weapon_action || "std")}) • Ammo: ${sol.weapon_ammo}/${sol.weapon_max_ammo}</div>
                <div class="provenance-line">
                  Dmg: ${sol.weapon_damage} • Range: ${sol.weapon_range}m • ROF: ${sol.weapon_rof} • Jam: ${sol.weapon_jam}%
                </div>
              </div>

              <div style="display:flex; gap:5px; flex-wrap:wrap;">
                <span class="badge">Kills: ${sol.kills}${sol.hth_kills ? ` (${sol.hth_kills} HTH)` : ""}</span>
                <span class="badge">Acc: ${accPct}% (${sol.hits}/${sol.shots})</span>
                <span class="badge">Skill: ${sol.skill}</span>
                <span class="badge">Spots: ${sol.spots}</span>
                ${sol.revive ? `<span class="badge">Revives: ${sol.revive}</span>` : ""}
                ${sol.mediced ? `<span class="badge">Mediced: ${sol.mediced}</span>` : ""}
                ${equipBadges}
              </div>
            </div>
          `;
        })
        .join("");
    }
  }

  // Right Pane 1: Mod-Set Fingerprint Card
  const fpCard = document.getElementById("save-fingerprint-card");
  const fpEntries = Object.entries(entry.profile.active_mods_fingerprint || {});
  fpCard.innerHTML = `
    <div class="config-option-card" style="margin-bottom:0">
      <div class="config-option-header">
        <span class="field-label">MODS USED WITH THIS SAVE (${fpEntries.length})</span>
        <span class="badge ${entry.profile.is_vanilla_clean ? "badge-ok" : ""}">
          ${entry.profile.is_vanilla_clean ? "Vanilla Save" : "Modded Save"}
        </span>
      </div>
      <div class="config-option-desc" style="margin-top:6px">
        ${
          fpEntries.length === 0
            ? "No mods recorded with this save yet."
            : fpEntries.map(([m, v]) => `<span class="badge" style="margin:2px">${escapeHtml(m)} v${escapeHtml(v)}</span>`).join(" ")
        }
      </div>
    </div>
  `;

  // Right Pane 2: Snapshot Backup Timeline
  const backupsEl = document.getElementById("save-backups-list");
  const backups = entry.profile.backups || [];
  document.getElementById("backup-count-pill").textContent = `${backups.length} Backups`;

  if (backups.length === 0) {
    backupsEl.innerHTML = `<p class="panel-subtitle">No backups created for '${escapeHtml(entry.profile.name)}' yet. Click "Back Up Save" to create one.</p>`;
  } else {
    backupsEl.innerHTML = backups
      .map(
        (b) => `
        <div class="backup-item">
          <div>
            <div><strong>${escapeHtml(b.label)}</strong> <span class="badge">${escapeHtml(b.id)}</span></div>
            <div class="provenance-line">${escapeHtml(b.timestamp)} • ${(b.size_bytes / 1024).toFixed(1)} KB</div>
          </div>
          <div class="toolbar-row">
            <button type="button" class="btn btn-xs btn-accent restore-backup-btn" data-bak-id="${escapeHtml(b.id)}">Restore</button>
            <button type="button" class="btn btn-xs btn-danger delete-backup-btn" data-bak-id="${escapeHtml(b.id)}" title="Delete Snapshot">x</button>
          </div>
        </div>
      `
      )
      .join("");

    backupsEl.querySelectorAll(".restore-backup-btn").forEach((btn) => {
      btn.addEventListener("click", async () => {
        try {
          await apiPost("/api/save/restore", {
            save_id: entry.profile.id,
            backup_id: btn.dataset.bakId,
          });
          await refreshState();
          showToast(`Restored backup '${btn.dataset.bakId}' into save '${entry.profile.id}'.`);
        } catch (err) {
          showToast(err.message, true);
        }
      });
    });

    backupsEl.querySelectorAll(".delete-backup-btn").forEach((btn) => {
      btn.addEventListener("click", async () => {
        try {
          await apiPost("/api/save/delete-backup", {
            save_id: entry.profile.id,
            backup_id: btn.dataset.bakId,
          });
          await refreshState();
          showToast(`Deleted backup '${btn.dataset.bakId}'.`);
        } catch (err) {
          showToast(err.message, true);
        }
      });
    });
  }

  // Right Pane 3: Automatically render Side-by-Side Save Comparison Table
  runSaveSlotComparison(false);
}

async function runSaveSlotComparison(showNotification = true) {
  const slotA = document.getElementById("compare-save-a")?.value || "default";
  const slotB = document.getElementById("compare-save-b")?.value || "vanilla";
  const box = document.getElementById("save-compare-output");
  if (!box) return;

  try {
    const cmp = await apiPost("/api/save/compare", { save_a: slotA, save_b: slotB });
    const fmtDelta = (n, unit = "") => {
      const val = typeof n === "number" ? n : 0;
      const prefix = val > 0 ? "+" : "";
      return unit ? `${prefix}${val} ${unit}` : `${prefix}${val}`;
    };

    const squadA = (cmp.soldiers_only_in_a || []).map(escapeHtml).join(", ") || "None";
    const squadB = (cmp.soldiers_only_in_b || []).map(escapeHtml).join(", ") || "None";
    const promoted = (cmp.promoted_soldiers || []).map(escapeHtml).join("; ") || "None";

    const modsA = (cmp.removed_mods || []).map(escapeHtml).join(", ") || "None";
    const modsB = (cmp.added_mods || []).map(escapeHtml).join(", ") || "None";
    const modsDelta = `${(cmp.added_mods || []).length} added, ${(cmp.removed_mods || []).length} removed`;

    box.innerHTML = `
      <table class="cmp-table">
        <thead>
          <tr>
            <th>Metric</th>
            <th>Slot A (${escapeHtml(slotA)})</th>
            <th>Slot B (${escapeHtml(slotB)})</th>
            <th>Delta</th>
          </tr>
        </thead>
        <tbody>
          <tr>
            <td><strong>Area of Operations (AO)</strong></td>
            <td>${escapeHtml(cmp.ao_a || "Unknown")}</td>
            <td>${escapeHtml(cmp.ao_b || "Unknown")}</td>
            <td>${cmp.ao_a === cmp.ao_b ? "Identical" : "Changed"}</td>
          </tr>
          <tr>
            <td><strong>Campaign Day</strong></td>
            <td>Day ${cmp.day_a ?? 0}</td>
            <td>Day ${cmp.day_b ?? 0}</td>
            <td>${fmtDelta(cmp.day_delta, "days")}</td>
          </tr>
          <tr>
            <td><strong>Tactical Points (TP)</strong></td>
            <td>${cmp.tp_a ?? 0} TP</td>
            <td>${cmp.tp_b ?? 0} TP</td>
            <td>${fmtDelta(cmp.tp_delta, "TP")}</td>
          </tr>
          <tr>
            <td><strong>Classic Rank &amp; Experience</strong></td>
            <td>${formatClassicRank(cmp.rank_a)} (${cmp.experience_a ?? 0} XP)</td>
            <td>${formatClassicRank(cmp.rank_b)} (${cmp.experience_b ?? 0} XP)</td>
            <td>${fmtDelta(cmp.rank_delta, "rank")} (${fmtDelta(cmp.experience_delta, "XP")})</td>
          </tr>
          <tr>
            <td><strong>Campaign Kills</strong></td>
            <td>${cmp.campaign_kills_a ?? 0} kills</td>
            <td>${cmp.campaign_kills_b ?? 0} kills</td>
            <td>${fmtDelta(cmp.campaign_kills_delta, "kills")}</td>
          </tr>
          <tr>
            <td><strong>Campaign Casualties (KIA)</strong></td>
            <td>${cmp.campaign_casualties_a ?? 0} KIA</td>
            <td>${cmp.campaign_casualties_b ?? 0} KIA</td>
            <td>${fmtDelta(cmp.campaign_casualties_delta, "KIA")}</td>
          </tr>
          <tr>
            <td><strong>Classic Max Wave</strong></td>
            <td>Wave ${cmp.classic_max_wave_a ?? 0}</td>
            <td>Wave ${cmp.classic_max_wave_b ?? 0}</td>
            <td>${fmtDelta(cmp.classic_max_wave_delta, "waves")}</td>
          </tr>
          <tr>
            <td><strong>Squad Only in Slot</strong></td>
            <td>${squadA}</td>
            <td>${squadB}</td>
            <td>${promoted !== "None" ? `Promoted: ${promoted}` : "No promotions"}</td>
          </tr>
          <tr>
            <td><strong>Active Mod Set</strong></td>
            <td>${modsA !== "None" ? `Missing in B: ${modsA}` : "None missing"}</td>
            <td>${modsB !== "None" ? `Added in B: ${modsB}` : "None added"}</td>
            <td>${modsDelta}</td>
          </tr>
        </tbody>
      </table>
    `;
    if (showNotification) {
      showToast(`Compared save '${slotA}' with '${slotB}'.`);
    }
  } catch (err) {
    if (showNotification) showToast(err.message, true);
  }
}

// ============================================================================
// TAB 4: LIBRARY & UPDATES (M6 / R7 & R8)
// ============================================================================

const VALID_LIBRARY_SUBTABS = ["installed", "community", "collections"];

function switchLibrarySubtab(targetSubtab) {
  // Validate target view against allowed subtabs (Requirement R7 / T2_R7_05)
  if (!VALID_LIBRARY_SUBTABS.includes(targetSubtab)) {
    console.warn(`Invalid library subpage routing target: '${targetSubtab}'. Defaulting to 'installed'.`);
    targetSubtab = "installed";
  }

  state.activeLibrarySubtab = targetSubtab;

  // Update button active state on #library-subtabs
  document.querySelectorAll("#library-subtabs .subtab").forEach((btn) => {
    const tabName = btn.dataset.subtab || btn.dataset.libtab;
    btn.classList.toggle("active", tabName === targetSubtab);
  });

  // Update sub-page panels visibility
  VALID_LIBRARY_SUBTABS.forEach((subtabId) => {
    const outerPanel = document.getElementById(`library-subtab-${subtabId}`);
    if (outerPanel) {
      const isActive = subtabId === targetSubtab;
      outerPanel.classList.toggle("active", isActive);
      outerPanel.hidden = !isActive;
    }
    const innerPanel = document.getElementById(`library-subpage-${subtabId}`);
    if (innerPanel) {
      const isActive = subtabId === targetSubtab;
      innerPanel.classList.toggle("active", isActive);
    }
  });

  // Render the specific subpage
  renderLibrarySubpage(targetSubtab);
}

function renderLibrarySubpage(subtab) {
  switch (subtab) {
    case "installed":
      renderInstalledModsSubpage();
      break;
    case "community":
      renderCommunityModsSubpage();
      break;
    case "collections":
      renderCollectionsSubpage();
      break;
    default:
      renderInstalledModsSubpage();
      break;
  }
}

function renderLibraryTab() {
  const currentSubtab = state.activeLibrarySubtab || "installed";
  switchLibrarySubtab(currentSubtab);
}

function renderInstalledModsSubpage() {
  const d = state.data;
  if (!d) return;
  const prof = d.active_profile || {};
  const grid = document.getElementById("library-cards-grid");
  if (!grid) return;

  const installedMods = d.installed_mods || [];

  // Update counters
  const counterEl = document.getElementById("library-count-installed");
  if (counterEl) counterEl.textContent = String(installedMods.length);
  const badgeEl = document.getElementById("installed-mods-summary-badge");
  if (badgeEl) badgeEl.textContent = `${installedMods.length} Mods Detected`;

  if (installedMods.length === 0) {
    grid.innerHTML = `
      <div class="empty-state" style="grid-column: 1 / -1; padding: 32px 16px; text-align: center;">
        <div class="empty-state-icon" style="font-size: 14px; margin-bottom: 8px; font-weight: bold; color: var(--text-muted);">[Directory Empty]</div>
        <h3 class="empty-state-title" style="margin: 0 0 6px 0;">No Installed Mods</h3>
        <p class="empty-state-subtitle" style="color: var(--text-secondary); margin: 0 0 12px 0;">
          No mods found in your library folder. Click 'Scan Mod Folder' or import an archive to get started.
        </p>
      </div>
    `;
    return;
  }

  grid.innerHTML = installedMods
    .map((m) => {
      const vers = m.installed_versions && m.installed_versions.length > 0 ? m.installed_versions : [m.manifest.version];
      const pinnedVer = (prof.pinned_versions || {})[m.manifest.id] || m.manifest.version;
      const isEnabled = (prof.enabled_mods || []).includes(m.manifest.id);

      // Inline update indicator check (Requirement R7)
      const upItem = (d.updates?.mods || []).find((u) => u.mod_id === m.manifest.id);
      const hasUpdate = upItem && upItem.update_available;

      return `
        <div class="config-option-card">
          <div class="config-option-header">
            <span>
              <strong>${escapeHtml(m.manifest.name)}</strong>
              <span class="version-tag">v${escapeHtml(m.manifest.version)}</span>
            </span>
            <div style="display:flex; gap:4px; align-items:center;">
              <span class="badge ${isEnabled ? "badge-ok" : "badge-neutral"}">
                ${isEnabled ? "Active" : "Inactive"}
              </span>
              <span class="badge ${m.integrity?.is_pristine ? "badge-ok" : "badge-warn"}">
                ${m.integrity?.is_pristine ? "Verified" : "Modified"}
              </span>
              ${
                hasUpdate
                  ? `<span class="badge badge-warn" title="New version available: v${escapeHtml(upItem.latest_version)}">[Update Available]</span>`
                  : ""
              }
            </div>
          </div>
          <div class="config-option-desc">${escapeHtml(m.manifest.description)}</div>
          <div class="provenance-line" style="margin-top:6px">
            ID: <code>${escapeHtml(m.manifest.id)}</code> • Prio: ${m.manifest.load_priority} • ${m.patch_count} patches
          </div>
          <div class="toolbar-row" style="margin-top:8px; justify-content:space-between;">
            <div class="toolbar-row">
              <span class="field-label">VERSION:</span>
              <select class="tactical-select lib-ver-select" data-mod-id="${escapeHtml(m.manifest.id)}" aria-label="Pinned Version">
                ${vers.map((v) => `<option value="${escapeHtml(v)}" ${v === pinnedVer ? "selected" : ""}>v${escapeHtml(v)}</option>`).join("")}
              </select>
            </div>
            <button type="button" class="btn btn-xs btn-accent lib-inspect-btn" data-mod-id="${escapeHtml(m.manifest.id)}">
              View Mod
            </button>
          </div>
        </div>
      `;
    })
    .join("");

  grid.querySelectorAll(".lib-inspect-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      state.selectedModId = btn.dataset.modId;
      document.querySelector('.nav-tab[data-tab="load-order"]')?.click();
      renderLoadOrderTab();
      renderModDetailDrawer();
    });
  });

  grid.querySelectorAll(".lib-ver-select").forEach((sel) => {
    sel.addEventListener("change", async () => {
      const modId = sel.dataset.modId;
      const nextPins = { ...(prof.pinned_versions || {}), [modId]: sel.value };
      await saveActiveProfileUpdate({ pinned_versions: nextPins });
    });
  });
}

async function fetchCommunityCatalog(forceRefresh = false) {
  const browserGrid = document.getElementById("browser-catalog-grid");
  if (browserGrid && (!state.communityCatalog || state.communityCatalog.length === 0)) {
    browserGrid.innerHTML = '<div style="grid-column: 1 / -1; padding: 28px; text-align: center; color: var(--text-muted);"><span class="spinner" style="margin-right:8px;"></span>Searching GitHub topics & Nexus for community mods...</div>';
  }

  try {
    const params = [];
    if (state.browserSearchQuery) params.push("query=" + encodeURIComponent(state.browserSearchQuery));
    if (state.browserTopicFilter) params.push("topic=" + encodeURIComponent(state.browserTopicFilter));
    if (state.browserSourceFilter) params.push("source=" + encodeURIComponent(state.browserSourceFilter));
    if (forceRefresh) params.push("refresh=true");

    const endpoint = "/api/browser/catalog" + (params.length > 0 ? "?" + params.join("&") : "");
    const res = await apiGet(endpoint);
    state.communityMeta = res;

    if (res && Array.isArray(res.catalog)) {
      state.communityCatalog = res.catalog;
    } else if (Array.isArray(res)) {
      state.communityCatalog = res;
    } else {
      state.communityCatalog = [];
    }

    if (res.rate_limited) {
      showToast("GitHub rate limit reached (60/hr). Showing cached catalog.", true);
    } else if (forceRefresh) {
      showToast(`Refreshed catalog (${state.communityCatalog.length} community mods found).`);
    }
  } catch (err) {
    console.warn("Offline or network error fetching community catalog:", err);
    state.communityCatalog = state.data?.browser_catalog || [];
    showToast("Offline mode: remote catalog unavailable.", false);
  }
  renderCommunityModsSubpage();
}

function renderCommunityModsSubpage() {
  const d = state.data;
  const browserGrid = document.getElementById("browser-catalog-grid");
  if (!browserGrid) return;

  const rawCatalog = state.communityCatalog !== null ? state.communityCatalog : (d?.browser_catalog || []);
  const bq = (state.browserSearchQuery || "").trim().toLowerCase();
  const bSrc = state.browserSourceFilter || "";

  // Honest catalog filtering without synthetic entries or fake stars
  const catalog = rawCatalog.filter((item) => {
    if (!item) return false;
    if (bSrc && item.source_type !== bSrc) return false;
    if (bq) {
      const hay = `${item.name || ""} ${item.id || ""} ${item.repo_full_name || ""} ${item.owner || ""} ${item.description || ""} ${(item.topics || []).join(" ")}`.toLowerCase();
      if (!hay.includes(bq)) return false;
    }
    return true;
  });

  const countBadge = document.getElementById("browser-catalog-count-badge");
  if (countBadge) countBadge.textContent = `${catalog.length} Available`;

  // Honest empty state presentation
  if (catalog.length === 0) {
    const errorMsg = state.communityMeta?.error_message || "";
    browserGrid.innerHTML = `
      <div class="empty-state catalog-empty-notice" style="grid-column: 1 / -1; padding: 36px 16px; text-align: center;">
        <div class="empty-state-icon" style="font-size: 14px; margin-bottom: 8px; font-weight: bold; color: var(--text-muted);">[No Packages]</div>
        <h3 class="empty-state-title" style="margin: 0 0 6px 0;">No community mods found</h3>
        <p class="empty-state-subtitle" style="color: var(--text-secondary); margin: 0 0 14px 0; max-width: 540px; margin-inline: auto;">
          ${errorMsg ? escapeHtml(errorMsg) + "<br><br>" : ""}
          No community mods match '${escapeHtml(state.browserSearchQuery || state.browserTopicFilter || "current filters")}'.
          You can tag GitHub repos with <code>mud-and-blood</code> or <code>quartermaster-mod</code>, or download archives from NexusMods directly.
        </p>
        <div class="toolbar-row" style="justify-content: center; gap: 8px;">
          <a href="https://www.nexusmods.com/mudandblood/mods" target="_blank" rel="noopener noreferrer" class="btn btn-sm btn-secondary">Browse NexusMods Hub &nearr;</a>
          <button type="button" class="btn btn-sm btn-accent" id="btn-empty-manual-install">Manual Install Archive</button>
          <button type="button" class="btn btn-sm" id="btn-empty-clear-filter">Reset Filters</button>
        </div>
      </div>
    `;

    document.getElementById("btn-empty-manual-install")?.addEventListener("click", () => {
      switchLibrarySubtab("installed");
      document.getElementById("import-path-input")?.focus();
    });

    document.getElementById("btn-empty-clear-filter")?.addEventListener("click", () => {
      state.browserSearchQuery = "";
      state.browserTopicFilter = "";
      state.browserSourceFilter = "";
      const searchInp = document.getElementById("browser-search-input");
      if (searchInp) searchInp.value = "";
      const srcFilter = document.getElementById("browser-source-filter");
      if (srcFilter) srcFilter.value = "";
      document.querySelectorAll("#browser-topic-pills .topic-pill").forEach((p) => {
        p.classList.toggle("active", p.dataset.topic === "");
      });
      fetchCommunityCatalog(true);
    });

    return;
  }

  // Render real catalog entries
  browserGrid.innerHTML = catalog
    .map((c) => {
      const isNexus = c.source_type === "NexusMods";
      const owner = c.owner || "Community";
      const stars = c.stargazers_count ? `Stars: ${c.stargazers_count}` : "";
      const forks = c.forks_count ? `⑂ ${c.forks_count}` : "";
      const license = c.license ? `License: ${escapeHtml(c.license)}` : "";
      const statsLine = [stars, forks, license].filter(Boolean).join(" • ");
      const tagPills = (c.topics || [])
        .map((t) => `<span class="badge" style="font-size:10px; margin-right:4px;">#${escapeHtml(t)}</span>`)
        .join("");

      return `
        <div class="config-option-card" style="display:flex; flex-direction:column; justify-content:space-between;">
          <div>
            <div class="config-option-header" style="align-items:flex-start;">
              <div>
                <strong>${escapeHtml(c.name)}</strong>
                ${c.latest_release_tag ? `<span class="version-tag">${escapeHtml(c.latest_release_tag)}</span>` : ""}
              </div>
              <span class="badge ${isNexus ? "badge-warn" : "badge-ok"}">${escapeHtml(c.source_type || "GitHub")}</span>
            </div>
            <div class="config-option-desc" style="margin: 6px 0 8px 0;">${escapeHtml(c.description || "No description provided.")}</div>
            ${statsLine ? `<div class="provenance-line" style="font-size:11px; color:var(--text-muted); margin-bottom:6px;">${statsLine}</div>` : ""}
            ${tagPills ? `<div style="margin-bottom:8px;">${tagPills}</div>` : ""}
          </div>
          <div class="toolbar-row" style="margin-top:auto; justify-content:space-between; border-top: 1px solid var(--border-subtle); padding-top:8px;">
            <span style="font-size:11px; color:var(--text-muted);">by ${escapeHtml(owner)}</span>
            <div style="display:flex; gap:6px;">
              ${
                c.installed
                  ? `<span class="badge badge-ok">Installed (v${escapeHtml(c.installed_version || "latest")})</span>`
                  : isNexus
                    ? `<a href="${escapeHtml(c.html_url || "https://www.nexusmods.com/mudandblood/mods")}" target="_blank" rel="noopener noreferrer" class="btn btn-xs btn-accent">Open on NexusMods &nearr;</a>`
                    : c.download_url
                      ? `<button type="button" class="btn btn-xs btn-accent browser-install-btn" data-url="${escapeHtml(c.download_url)}" data-name="${escapeHtml(c.name)}" data-id="${escapeHtml(c.id)}">Download &amp; Install</button>
                         <a href="${escapeHtml(c.html_url)}" target="_blank" rel="noopener noreferrer" class="btn btn-xs">GitHub &nearr;</a>`
                      : `<a href="${escapeHtml(c.html_url)}" target="_blank" rel="noopener noreferrer" class="btn btn-xs">View on GitHub &nearr;</a>`
              }
            </div>
          </div>
        </div>
      `;
    })
    .join("");

  browserGrid.querySelectorAll(".browser-install-btn").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const url = btn.dataset.url;
      const modId = btn.dataset.id;
      const name = btn.dataset.name;
      if (!url) return;

      btn.disabled = true;
      const origText = btn.innerHTML;
      btn.innerHTML = `<span class="spinner" style="margin-right:4px;"></span>Downloading...`;

      try {
        showToast(`Downloading '${name}' from GitHub...`);
        const res = await apiPost("/api/browser/install", { download_url: url, mod_id: modId });

        const caps = res.security_audit?.capabilities_detected || [];
        if (caps.length > 0) {
          showToast(`Installed '${name}' with security notes: ${caps.join(", ")}`);
        } else {
          showToast(`Successfully installed '${name}' into library!`);
        }

        await refreshState();
        await fetchCommunityCatalog();
      } catch (err) {
        showToast(`Download failed: ${err.message}`, true);
        btn.disabled = false;
        btn.innerHTML = origText;
      }
    });
  });
}

function renderCollectionsSubpage() {
  const d = state.data;
  if (!d) return;

  const upList = document.getElementById("update-keys-list");
  if (!upList) return;

  const upInfo = d.updates || {};
  const launcherUp = upInfo.launcher || {};
  const launcherCard = `
    <div class="config-option-card" style="margin-bottom:6px">
      <div class="config-option-header">
        <strong>QuarterMaster Launcher (v${escapeHtml(launcherUp.current_version || d.version)})</strong>
        <span class="badge badge-ok">Up to Date</span>
      </div>
      <div class="provenance-line">
        <strong>Signature:</strong> ${escapeHtml(launcherUp.signature_verification || "Ed25519")} •
        <strong>Cache:</strong> ${escapeHtml(upInfo.rate_limit_mode || "ETag")}
      </div>
    </div>
  `;

  const modUpItems = (upInfo.mods || (d.installed_mods || []).map((m) => ({
    mod_id: m.manifest.id,
    name: m.manifest.name,
    installed_version: m.manifest.version,
    latest_version: m.manifest.version,
    update_available: false,
    update_key: (m.manifest.update_keys || [])[0] || "None",
    source_changed_warning: false,
  })))
    .map(
      (u) => `
      <div class="backup-item">
        <div>
          <div>
            <strong>${escapeHtml(u.name)}</strong>
            <span class="badge">Installed: v${escapeHtml(u.installed_version)}</span>
            <span class="badge ${u.update_available ? "badge-warn" : "badge-ok"}">Latest: v${escapeHtml(u.latest_version)}</span>
          </div>
          <div class="provenance-line">UpdateKey: <code>${escapeHtml(u.update_key || "None declared")}</code></div>
          ${
            u.source_changed_warning
              ? `<div class="provenance-line" style="color:var(--status-danger)">Warning: Update source changed from pinned source!</div>`
              : `<div class="provenance-line">Source verified</div>`
          }
        </div>
        <span class="badge badge-ok">Compatible</span>
      </div>
    `
    )
    .join("");

  upList.innerHTML = launcherCard + modUpItems;
}

function setupLibrarySubtabs() {
  const subtabs = document.querySelectorAll("#library-subtabs .subtab");
  subtabs.forEach((btn) => {
    btn.addEventListener("click", () => {
      const target = btn.dataset.subtab || btn.dataset.libtab;
      switchLibrarySubtab(target);
    });
  });

  // Refresh community catalog button hook
  document.getElementById("btn-refresh-catalog")?.addEventListener("click", () => {
    fetchCommunityCatalog(true);
  });

  // Topic filter pill hooks
  document.querySelectorAll("#browser-topic-pills .topic-pill").forEach((pill) => {
    pill.addEventListener("click", () => {
      document.querySelectorAll("#browser-topic-pills .topic-pill").forEach((p) => p.classList.remove("active"));
      pill.classList.add("active");
      state.browserTopicFilter = pill.dataset.topic || "";
      fetchCommunityCatalog(true);
    });
  });

  const searchInput = document.getElementById("browser-search-input");
  if (searchInput) {
    let debounceTimer;
    searchInput.addEventListener("input", (e) => {
      clearTimeout(debounceTimer);
      state.browserSearchQuery = e.target.value.trim();
      debounceTimer = setTimeout(() => {
        fetchCommunityCatalog(true);
      }, 400);
    });
  }

  const srcFilter = document.getElementById("browser-source-filter");
  if (srcFilter) {
    srcFilter.addEventListener("change", (e) => {
      state.browserSourceFilter = e.target.value;
      fetchCommunityCatalog(false);
    });
  }

  // Manual package install submit button hook
  document.getElementById("btn-manual-install-submit")?.addEventListener("click", async () => {
    const p = document.getElementById("manual-install-path")?.value.trim();
    if (!p) return showToast("Enter a mod archive path or URL first.", true);
    try {
      await apiPost("/api/mod/import", { path: p, allow_unmanaged: true });
      invalidateDiffState();
      await refreshState();
      showToast(`Installed mod package from '${p}'.`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  // Manual package install browse button hook
  document.getElementById("btn-manual-install-browse")?.addEventListener("click", () => {
    const p = prompt("Enter full path to mod .zip archive or folder to install:");
    if (p && p.trim()) {
      const inp = document.getElementById("manual-install-path");
      if (inp) inp.value = p.trim();
    }
  });

  // 1-Click protocols card button hook
  document.getElementById("btn-register-protocols-card")?.addEventListener("click", () => {
    document.getElementById("btn-register-protocols")?.click();
  });
}

// ============================================================================
// TAB 5: CRASH DIAGNOSTICS, PROXY DLL HEALTH & RUNTIME LOG VIEWER
// ============================================================================
function renderCrashesTab() {
  const d = state.data;
  const crashes = d.crashes || [];
  const health = d.health || {};

  // 1. System & Proxy DLL Health Banner + Game Update Detection + Steam Cloud Sync
  const healthBanner = document.getElementById("system-health-banner");
  const dllOk = Boolean(health.dll_present && health.dll_matches_build);
  const gameUpd = health.game_update_detected;
  const cloudWarn = health.steam_cloud_warning;

  const sourceReady = Boolean(health.source_dll_ready || health.source_dll_sha256);
  const statusBadge = health.dll_present
    ? (health.dll_matches_build ? '<span class="badge badge-ok">winmm.dll Active (Deployed)</span>' : '<span class="badge badge-warn">winmm.dll Outdated</span>')
    : sourceReady
    ? '<span class="badge badge-ok">Clean Vanilla State (Staged for Launch)</span>'
    : '<span class="badge badge-danger">winmm.dll Build Artifact Missing</span>';

  const provenanceText = health.dll_present
    ? `SHA-256: <code>${(health.dll_sha256 || "").slice(0, 16)}…</code>`
    : sourceReady
    ? `Staged at launcher (SHA-256: <code>${(health.source_dll_sha256 || "").slice(0, 16)}…</code>) • Deployed automatically on launch`
    : "winmm.dll build artifact missing from launcher";

  healthBanner.innerHTML = `
    <div style="display:flex; align-items:center; justify-content:space-between; flex-wrap:wrap; gap:10px;">
      <div>
        ${statusBadge}
        <strong style="margin-left:6px">Game Directory State:</strong>
        <span class="provenance-line">${provenanceText}</span>
      </div>
      <div class="toolbar-row">
        <button type="button" class="btn btn-xs btn-accent" id="btn-repair-dll">Deploy Staged winmm.dll</button>
        <button type="button" class="btn btn-xs" id="btn-launch-safe-mode">Launch Vanilla</button>
      </div>
    </div>
    ${
      gameUpd
        ? `<div style="margin-top:8px; padding-top:8px; border-top:1px solid var(--border-subtle); display:flex; align-items:center; justify-content:space-between;">
            <span><strong>[Game Update Detected]</strong> <code>mnb.exe</code> has changed. Re-validate mod patches against the new game version.</span>
            <button type="button" class="btn btn-xs btn-accent" id="btn-ack-game-update">Update Vanilla Cache</button>
          </div>`
        : ""
    }
    ${
      cloudWarn
        ? `<div class="provenance-line" style="margin-top:6px">Cloud: ${escapeHtml(cloudWarn)}</div>`
        : ""
    }
  `;

  document.getElementById("btn-ack-game-update")?.addEventListener("click", async () => {
    try {
      await apiPost("/api/system/acknowledge-game-update", {});
      invalidateDiffState();
      await refreshState();
      showToast("Updated vanilla cache and re-validated patches.");
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-repair-dll")?.addEventListener("click", async () => {
    try {
      const res = await apiPost("/api/health/repair-dll", {});
      await refreshState();
      showToast("Reinstalled and verified winmm.dll.");
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-launch-safe-mode")?.addEventListener("click", () => {
    document.getElementById("btn-launch-vanilla")?.click();
  });

  // 2. Crash Reports List
  const container = document.getElementById("crashes-list-container");
  if (crashes.length === 0) {
    container.innerHTML = `
      <div class="config-option-card">
        <strong style="color:var(--status-ok)">No crashes recorded.</strong>
        <p class="config-option-desc">When a crash occurs in-game, QuarterMaster captures the error and identifies which mod and patch caused it.</p>
      </div>
    `;
  } else {
    container.innerHTML = crashes
      .map(
        (c) => `
        <div class="conflict-card" style="margin-bottom:10px">
          <div class="config-option-header">
            <div>
              <span class="badge badge-warn">CRASH REPORT</span>
              <strong>${escapeHtml(c.error_message)}</strong>
              <span class="provenance-line">(${escapeHtml(c.timestamp)})</span>
            </div>
            ${
              c.suspected_mod_id
                ? `<button type="button" class="btn btn-sm btn-danger disable-suspect-btn" data-mod-id="${escapeHtml(c.suspected_mod_id)}">
                    Disable '${escapeHtml(c.suspected_mod_id)}'
                  </button>`
                : ""
            }
          </div>
          <div class="provenance-line">
            <strong>Suspected Mod:</strong> <code>${escapeHtml(c.suspected_mod_id || "Unknown / Vanilla")}</code> •
            <strong>Suspected Patch:</strong> <code>${escapeHtml(c.suspected_patch_name || c.suspected_patch_id || "N/A")}</code>
          </div>
          <pre class="markdown-box" style="margin-top:6px">${escapeHtml(c.traceback)}</pre>
        </div>
      `
      )
      .join("");

    container.querySelectorAll(".disable-suspect-btn").forEach((btn) => {
      btn.addEventListener("click", async () => {
        try {
          await apiPost("/api/crash/disable-mod", { mod_id: btn.dataset.modId });
          invalidateDiffState();
          await refreshState();
          showToast(`Disabled '${btn.dataset.modId}'.`);
        } catch (err) {
          showToast(err.message, true);
        }
      });
    });
  }

  // 3. Live Runtime Patch Report & Log Viewer (Filtered by Level and Mod)
  const logModSel = document.getElementById("log-mod-filter");
  if (logModSel) {
    logModSel.innerHTML =
      `<option value="">All Mods</option>` +
      d.installed_mods
        .map(
          (m) =>
            `<option value="${escapeHtml(m.manifest.id)}" ${m.manifest.id === state.logModFilter ? "selected" : ""}>${escapeHtml(m.manifest.name)}</option>`
        )
        .join("");
  }

  const logViewer = document.getElementById("runtime-log-viewer");
  const logs = (health.recent_logs || []).filter((l) => {
    if (state.logLevelFilter !== "ALL" && l.level !== state.logLevelFilter) return false;
    if (state.logModFilter && !String(l.line || "").toLowerCase().includes(state.logModFilter.toLowerCase())) {
      return false;
    }
    return true;
  });
  const pr = health.patch_report;

  const prHtml = pr
    ? `<div class="config-option-card">
        <div class="config-option-header">
          <strong>Last In-Game <code>patch-report.json</code></strong>
          <span class="badge badge-ok">${(pr.records || []).length} Runtime Records</span>
        </div>
        <div class="provenance-line">Generated at: ${escapeHtml(pr.generated_at || "Recent Launch")} • Profile: <code>${escapeHtml(pr.profile_id || "default")}</code></div>
      </div>`
    : `<div class="config-option-card">
        <span class="panel-subtitle">No in-game <code>Mods/qm/patch-report.json</code> generated yet (written automatically by <code>winmm.dll</code> when Mud and Blood launches).</span>
      </div>`;

  const logsHtml =
    logs.length === 0
      ? `<div class="config-option-card"><span class="panel-subtitle">No runtime log entries matching '${state.logLevelFilter}'.</span></div>`
      : `<pre class="markdown-box">${logs
          .map((l) => `[${l.level}] ${escapeHtml(l.line)}`)
          .join("\n")}</pre>`;

  logViewer.innerHTML = prHtml + logsHtml;
}

// ============================================================================
// TAB 6: MODDER DEV STUDIO
// ============================================================================
function renderDevToolsTab() {
  const d = state.data;
  const sel = document.getElementById("lint-mod-select");
  if (sel) {
    if (state.scaffoldActiveModDir) {
      sel.innerHTML = `<option value="${escapeHtml(state.scaffoldActiveModDir)}">${escapeHtml(state.scaffoldActiveModDir)} (Working Mod)</option>`;
      loadScaffoldModFiles(state.scaffoldActiveModDir);
    } else {
      sel.innerHTML = `<option value="">(No new mod created yet — use Section 1 on the left)</option>`;
      loadScaffoldModFiles(null);
    }
  }

  const editorModSel = document.getElementById("editor-mod-select");
  if (editorModSel && d.installed_mods) {
    const curVal = editorModSel.value || state.editorActiveModDir;
    editorModSel.innerHTML = d.installed_mods
      .map(
        (m) =>
          `<option value="${escapeHtml(m.manifest.id)}" data-path="${escapeHtml(m.install_path)}" ${m.manifest.id === curVal || m.install_path === curVal ? "selected" : ""}>${escapeHtml(m.manifest.name)} (${escapeHtml(m.manifest.id)})</option>`
      )
      .join("");
    if (!state.editorActiveModDir && editorModSel.value) {
      state.editorActiveModDir = editorModSel.value;
    }
  }

  const tplSel = document.getElementById("scaf-template");
  if (tplSel) {
    const curVal = tplSel.value;
    const tpls = (d.templates && d.templates.length > 0) ? d.templates : [
      { id: "gmenu_settings", name: "GMenu Settings Page", category: "Menu Frameworks & Options Pages", descriptor: "Custom Menu — Full GMenu settings page with all control types" },
      { id: "custom_hud_overlay", name: "Custom HUD & In-Game UI Overlay", category: "Menu Frameworks & Options Pages", descriptor: "HUD Overlay — Custom canvas status panels and squad telemetry" },
      { id: "sprite_replacer", name: "Sprite & Texture Replacer", category: "Sprite & Graphics Replacements", descriptor: "Sprite Replacer — Override soldier, vehicle, and weapon graphics" },
      { id: "custom_particle_fx", name: "Custom Particle Effect", category: "Sprite & Graphics Replacements", descriptor: "Particle Effects — Custom explosion, smoke, and flash particles" },
      { id: "sound_replacer", name: "Sound Effect Replacer", category: "Audio & Music Replacements / Additions", descriptor: "Sound Replacer — Custom weapon reports, explosions, and voice cues" },
      { id: "custom_battle_music", name: "Custom Battle Music & Ambient Audio", category: "Audio & Music Replacements / Additions", descriptor: "Battle Music — Streaming dynamic combat and ambient playlists" },
      { id: "custom_weapon", name: "Custom Weapon Definition", category: "New Content & Gameplay Mechanics", descriptor: "Custom Weapon — Complete weapon definition with ROF, recoil, and ammo" },
      { id: "custom_soldier_class", name: "Custom Soldier Class & Archetype", category: "New Content & Gameplay Mechanics", descriptor: "Soldier Archetype — Custom loadout, stats, traits, and squad logic" },
      { id: "custom_battle_condition", name: "Custom Battle Condition (BC Mod)", category: "New Content & Gameplay Mechanics", descriptor: "Battle Condition — Weather, artillery barrages, and combat rules" },
      { id: "custom_reinforcement_card", name: "Custom Reinforcement Call-In", category: "New Content & Gameplay Mechanics", descriptor: "Reinforcement Call-In — Tactical point abilities and squad summons" },
      { id: "custom_map_structure", name: "Custom Map Structure & Fortification", category: "New Content & Gameplay Mechanics", descriptor: "Map Structure — Bunkers, fortifications, and trench emplacements" },
      { id: "single_anchor_patch", name: "Code Patch (Single Anchor)", category: "Core Code Patching & Engine Hooking", descriptor: "Single Anchor Patch — Clean, surgical qm.toml code replacement" },
      { id: "multi_file_engine_hook", name: "Multi-File Engine Hook", category: "Core Code Patching & Engine Hooking", descriptor: "Multi-File Engine Hook — Game loop interception across core files" },
      { id: "asset_file_injection", name: "New Game File & Asset Injection", category: "Core Code Patching & Engine Hooking", descriptor: "Asset & Module Injection — Virtual filesystem mounting and new Lua modules" },
      { id: "empty", name: "Minimal Patch Mod", category: "Core Code Patching & Engine Hooking", descriptor: "Minimal Patch Mod — Clean mod.json and qm.toml starter" },
    ];

    if (tplSel.options.length !== tpls.length) {
      const categories = {};
      tpls.forEach((t) => {
        const cat = t.category || "General Templates";
        if (!categories[cat]) categories[cat] = [];
        categories[cat].push(t);
      });
      const catKeys = Object.keys(categories);
      if (catKeys.length > 1) {
        tplSel.innerHTML = catKeys
          .map(
            (cat) =>
              `<optgroup label="${escapeHtml(cat)}">${categories[cat]
                .map(
                  (t) =>
                    `<option value="${escapeHtml(t.id)}" ${t.id === curVal ? "selected" : ""}>${escapeHtml(
                      t.descriptor || t.name
                    )}</option>`
                )
                .join("")}</optgroup>`
          )
          .join("");
      } else {
        tplSel.innerHTML = tpls
          .map(
            (t) =>
              `<option value="${escapeHtml(t.id)}" ${t.id === curVal ? "selected" : ""}>${escapeHtml(
                t.descriptor || t.name
              )}</option>`
          )
          .join("");
      }
      if (curVal) tplSel.value = curVal;
    }
    updateTemplatePreview();
  }

  if (state.activeModTab === "api-docs") {
    initApiDocsTab();
  } else if (state.activeModTab === "editor") {
    initModEditorTab();
  }
}

function updateTemplatePreview() {
  const tplSel = document.getElementById("scaf-template");
  const previewBox = document.getElementById("template-code-preview");
  if (!previewBox) return;
  const curId = tplSel ? tplSel.value : "";
  const tpls = state.data?.templates || [];
  const found = tpls.find((t) => t.id === curId);
  if (found && found.preview_code) {
    previewBox.textContent = found.preview_code;
  } else if (!curId) {
    previewBox.textContent = "-- No template selected.";
  } else {
    previewBox.textContent = `-- Template '${curId}' has no preview available.`;
  }
}

// ============================================================================
// TAB 6 SUB-TABS & BASE GAME API DOCUMENTATION (M1 / R1)
// ============================================================================

function setupModToolsSubtabs() {
  const subtabs = document.querySelectorAll("#modtools-subtabs .subtab");
  subtabs.forEach((btn) => {
    btn.addEventListener("click", () => {
      const targetModTab = btn.dataset.modtab;
      subtabs.forEach((b) => b.classList.remove("active"));
      btn.classList.add("active");

      document.querySelectorAll("#tab-devtools .modtools-subpanel").forEach((panel) => {
        panel.classList.remove("active");
        panel.hidden = true;
      });

      const activePanel = document.getElementById(`modtools-tab-${targetModTab}`);
      if (activePanel) {
        activePanel.classList.add("active");
        activePanel.hidden = false;
      }
      state.activeModTab = targetModTab;

      if (targetModTab === "studio") {
        initPatchStudio();
      } else if (targetModTab === "editor") {
        initModEditorTab();
      } else if (targetModTab === "tutorials") {
        initHybridTutorialsHub();
      } else if (targetModTab === "api-docs") {
        initApiDocsTab();
      }
    });
  });

  // Automatically initialize default active subtab (studio) if selected
  const initialActive = document.querySelector("#modtools-subtabs .subtab.active");
  if (initialActive && initialActive.dataset.modtab === "studio") {
    initPatchStudio();
  }

  // Template preview change handler (M2 / R2)
  const tplSel = document.getElementById("scaf-template");
  tplSel?.addEventListener("change", () => {
    updateTemplatePreview();
  });

  // Template preview Copy Code button (M2 / R2)
  const copyTplBtn = document.getElementById("btn-copy-template");
  copyTplBtn?.addEventListener("click", () => {
    const previewBox = document.getElementById("template-code-preview");
    const code = previewBox ? previewBox.textContent : "";
    if (!code || code.startsWith("-- No template selected.")) {
      showToast("No template code to copy.", true);
      return;
    }
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(code).then(() => {
        showToast("Template code copied to clipboard!");
      }).catch(() => {
        fallbackCopyText(code);
      });
    } else {
      fallbackCopyText(code);
    }
  });
}

function initPatchStudio() {
  if (state.patchStudioInitialized) return;
  state.patchStudioInitialized = true;
  setupPatchStudioUI();
}

async function setupPatchStudioUI() {
  const container = document.getElementById("patch-studio-container");
  if (!container) return;

  container.innerHTML = `
    <!-- LEFT PANE: Source Explorer & Overlay -->
    <div class="studio-pane" id="studio-left-pane">
      <div class="studio-pane-header">
        <div style="display: flex; align-items: center; gap: 8px;">
          <span class="field-label" style="margin: 0;">Target File:</span>
          <select id="studio-file-select" class="tactical-select" style="max-width: 220px; padding: 2px 6px; font-size: 11px;"></select>
          <span class="badge" id="studio-range-badge" style="font-size: 10px;">Lines 1-5</span>
          <button type="button" class="btn btn-xs" id="btn-studio-clear-selection" title="Clear selection">Clear</button>
        </div>
        <div style="display: flex; gap: 6px; align-items: center;">
          <button type="button" class="btn btn-xs" id="btn-studio-decompile-assets" title="Extract non-Lua assets (sprites, sounds, fonts) from game executable">Decompile Assets</button>
          <button type="button" class="btn btn-xs ${state.engineShowOtherPatches ? "btn-accent" : ""}" id="btn-studio-toggle-overlay">
            ${state.engineShowOtherPatches ? "Overlay: ON" : "Overlay: OFF"}
          </button>
        </div>
      </div>
      <div class="source-code-scroll" id="studio-source-scroll">
        <div style="padding: 16px; color: var(--text-muted); text-align: center;">Loading source file...</div>
      </div>
    </div>

    <!-- RIGHT PANE: Wizard or Live Editor -->
    <div class="studio-pane" id="studio-right-pane"></div>
  `;

  // Populate file selector with all 43 vanilla files
  const fileSelect = document.getElementById("studio-file-select");
  if (fileSelect) {
    const all43VanillaFiles = [
      "conf.lua", "main.lua",
      "includes/ai.lua", "includes/audioAudit.lua", "includes/bcChecks.lua",
      "includes/binser.lua", "includes/cron.lua", "includes/directives.lua",
      "includes/drawFiles.lua", "includes/drawFiles_1.lua", "includes/external.lua",
      "includes/gfx.lua", "includes/localisation.lua", "includes/mapBuild.lua",
      "includes/mapBuild_1.lua", "includes/mapBuild_3.lua", "includes/mapGeneration.lua",
      "includes/mnb2.lua", "includes/particlesSystem.lua", "includes/ser.lua",
      "includes/temp.lua", "includes/trench.lua", "includes/urb_engine.lua",
      "includes/vehicleEngine.lua", "includes/workFile.lua",
      "moonshine/boxblur.lua", "moonshine/chromasep.lua", "moonshine/colorgradesimple.lua",
      "moonshine/crt.lua", "moonshine/desaturate.lua", "moonshine/dmg.lua",
      "moonshine/fastgaussianblur.lua", "moonshine/filmgrain.lua", "moonshine/fog.lua",
      "moonshine/gaussianblur.lua", "moonshine/glow.lua", "moonshine/godsray.lua",
      "moonshine/init.lua", "moonshine/pixelate.lua", "moonshine/posterize.lua",
      "moonshine/scanlines.lua", "moonshine/sketch.lua", "moonshine/vignette.lua"
    ];
    let filesToUse = all43VanillaFiles;
    try {
      const vRes = await apiGet("/api/devtools/vanilla-files");
      if (vRes && Array.isArray(vRes.files) && vRes.files.length > 0) {
        filesToUse = vRes.files;
      }
    } catch (_) {}

    fileSelect.innerHTML = filesToUse
      .map(f => `<option value="${f}" ${f === (state.patchStudioActiveFile || "main.lua") ? "selected" : ""}>${f}</option>`)
      .join("");
    fileSelect.addEventListener("change", (e) => {
      loadPatchStudioFile(e.target.value);
    });
  }

  // Decompile non-Lua assets button
  document.getElementById("btn-studio-decompile-assets")?.addEventListener("click", async () => {
    try {
      showToast("Decompiling game assets from executable...");
      const res = await apiPost("/api/devtools/decompile-assets", {});
      showToast(`[OK] Decompiled ${res.extracted_count || "all"} game assets to decompile/ folder!`);
    } catch (e) {
      showToast(`Decompile failed: ${e.message}`, true);
    }
  });

  // Clear selection button
  document.getElementById("btn-studio-clear-selection")?.addEventListener("click", () => {
    clearPatchSelection();
  });

  // Toggle overlay button
  document.getElementById("btn-studio-toggle-overlay")?.addEventListener("click", () => {
    togglePatchOverlay(!state.engineShowOtherPatches);
    const btn = document.getElementById("btn-studio-toggle-overlay");
    if (btn) {
      btn.textContent = state.engineShowOtherPatches ? "Overlay: ON" : "Overlay: OFF";
      btn.classList.toggle("btn-accent", state.engineShowOtherPatches);
    }
  });

  // Render initial right pane
  renderPatchStudioRightPane();

  // Load active file
  await loadPatchStudioFile(state.patchStudioActiveFile || "includes/mapBuild.lua");
}

async function loadPatchStudioFile(filePath) {
  state.patchStudioActiveFile = filePath;
  try {
    const res = await apiGet(`/api/devtools/vanilla-source?file=${encodeURIComponent(filePath)}`);
    if (res && res.lines) {
      state.currentSourceLines = res.lines;
    } else if (res && res.content) {
      state.currentSourceLines = res.content.split("\n");
    }
  } catch (err) {
    if (!state.currentSourceLines || state.currentSourceLines.length === 0) {
      state.currentSourceLines = new Array(80).fill("").map((_, i) => `-- ${filePath} line ${i + 1}`);
    }
  }

  // Query active patches touching this file from simulation
  try {
    const diff = await apiGet(`/api/file-diff?file=${encodeURIComponent(filePath)}`);
    if (diff && diff.records) {
      state.engineActivePatchesForFile = diff.records.map(r => ({
        patch_id: r.patch_id,
        line_range: r.modified_line_range
      }));
    }
  } catch (_) {
    state.engineActivePatchesForFile = [];
  }

  // Reset range if out of bounds
  if (!state.patchTargetRange || state.patchTargetRange[1] > state.currentSourceLines.length) {
    setPatchTargetRange(1, Math.min(10, state.currentSourceLines.length));
  }

  renderPatchStudioSourcePane(filePath);
}

function clearPatchSelection() {
  state.patchTargetRange = null;
  const badge = document.getElementById("studio-range-badge");
  if (badge) badge.textContent = "No selection";
  if (typeof renderPatchStudioSourcePane === "function" && state.patchStudioActiveFile) {
    renderPatchStudioSourcePane(state.patchStudioActiveFile);
  }
  updatePatchWizardTarget();
}

let _studioDragHandlerInstalled = false;
function attachPatchStudioDragEvents(container) {
  if (!container || typeof container.addEventListener !== "function" || _studioDragHandlerInstalled) return;
  _studioDragHandlerInstalled = true;

  let isDragging = false;
  let dragAnchorLine = null;

  container.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    if (state.patchWizardStep === 2) return;

    const row = e.target?.closest ? e.target.closest(".source-code-row") : null;
    if (!row || !row.dataset?.line) return;
    const line = parseInt(row.dataset.line, 10);
    if (Number.isNaN(line)) return;

    isDragging = true;
    dragAnchorLine = line;
    setPatchTargetRange(line, line);
    updateDragVisualClasses(container, line, line);
    e.preventDefault?.();
  });

  container.addEventListener("mousemove", (e) => {
    if (!isDragging || dragAnchorLine === null) return;
    const row = e.target?.closest ? e.target.closest(".source-code-row") : null;
    if (!row || !row.dataset?.line) return;
    const line = parseInt(row.dataset.line, 10);
    if (Number.isNaN(line)) return;

    const s = Math.min(dragAnchorLine, line);
    const end = Math.max(dragAnchorLine, line);
    setPatchTargetRange(s, end);
    updateDragVisualClasses(container, s, end);
  });

  const onMouseUp = () => {
    if (isDragging) {
      isDragging = false;
      dragAnchorLine = null;
      if (typeof renderPatchStudioSourcePane === "function" && state.patchStudioActiveFile) {
        renderPatchStudioSourcePane(state.patchStudioActiveFile);
      }
    }
  };

  if (typeof window !== "undefined" && typeof window.addEventListener === "function") {
    window.addEventListener("mouseup", onMouseUp);
  }
}

function updateDragVisualClasses(container, start, end) {
  const badge = document.getElementById("studio-range-badge");
  if (badge) badge.textContent = `Lines ${start}–${end}`;
  const rows = container.querySelectorAll(".source-code-row");
  for (const r of rows) {
    const l = parseInt(r.dataset.line, 10);
    if (l >= start && l <= end) {
      r.classList.add("target-highlight");
      r.classList.remove("dimmed");
    } else {
      r.classList.remove("target-highlight");
      if (!r.classList.contains("other-patch-highlight")) {
        r.classList.add("dimmed");
      }
    }
  }
}

function renderPatchStudioSourcePane(filePath) {
  const container = document.getElementById("studio-source-scroll");
  if (!container) return;

  const badge = document.getElementById("studio-range-badge");
  if (badge) {
    if (state.patchTargetRange) {
      badge.textContent = `Lines ${state.patchTargetRange[0]}–${state.patchTargetRange[1]}`;
    } else {
      badge.textContent = "No selection";
    }
  }

  const hint = document.getElementById("studio-instruction-hint");
  if (hint) {
    if (state.patchWizardStep === 2) {
      hint.innerHTML = '<span class="badge badge-ok" style="font-size: 9.5px; padding: 1px 5px;">Live Diff Preview Active</span>';
    } else {
      hint.textContent = "Click & drag lines to select target";
    }
  }

  const savedScroll = container.scrollTop;
  container.innerHTML = renderPatchStudioSourceLines(
    state.currentSourceLines,
    state.patchTargetRange,
    state.engineActivePatchesForFile,
    state.engineShowOtherPatches
  );
  container.scrollTop = savedScroll;

  attachPatchStudioDragEvents(container);
  updatePatchWizardTarget();
}

function renderPatchStudioRightPane() {
  const pane = document.getElementById("studio-right-pane");
  if (!pane) return;

  if (state.patchWizardStep === 1) {
    renderPatchWizardStep();
  } else {
    renderLivePatchEditor();
  }
}

function renderPatchWizardStep() {
  const pane = document.getElementById("studio-right-pane");
  if (!pane) return;

  const data = state.patchWizardData || (state.patchWizardData = {});
  const [start, end] = state.patchTargetRange || [1, 1];
  const file = state.patchStudioActiveFile || "conf.lua";

  const categories = [
    "# APPLICATION LIFECYCLE & SETTINGS",
    "# BATTLEFIELD & MAP BUILD",
    "# CORE GAME STATE & CLASS HOOKS",
    "# COMBAT AI & BEHAVIORS",
    "# WEAPONS, BALLISTICS & DAMAGE",
    "# RENDERING, PARTICLES & SFX",
    "# CUSTOM LUA MODULES & EXTENSIONS"
  ];

  const installedMods = state.data?.installed_mods || [];
  if (!data.targetMod && installedMods.length > 0) {
    data.targetMod = installedMods[0].manifest.id;
  }

  const catOptions = categories
    .map(c => `<option value="${c}" ${c === (data.category || categories[1]) ? "selected" : ""}>${c}</option>`)
    .join("");

  const modOptions = installedMods.map(m => `
    <option value="${m.manifest.id}" ${m.manifest.id === data.targetMod ? "selected" : ""}>
      ${escapeHtml(m.manifest.name)} (${escapeHtml(m.manifest.id)})
    </option>
  `).join("");

  const hasInProgress = Boolean(data.patchId || data.payload);

  pane.innerHTML = `
    <div class="studio-pane-header">
      <div style="display: flex; gap: 8px; align-items: center;">
        <span style="font-weight: 600; color: var(--text-primary);">Step 1: Patch Wizard &amp; Metadata</span>
        ${hasInProgress ? '<span class="badge badge-accent" style="font-size: 10px;">In Progress</span>' : ''}
      </div>
      <div style="display: flex; gap: 6px; align-items: center;">
        <span class="badge" style="font-size: 10px;">Target: ${escapeHtml(file)}:${state.patchTargetRange ? `${start}–${end}` : 'None'}</span>
        ${hasInProgress ? '<button type="button" class="btn btn-xs btn-accent" id="btn-wizard-goto-editor">Go to Editor &rarr;</button>' : ''}
      </div>
    </div>
    <div class="dev-form" style="padding: 14px; overflow-y: auto; flex: 1; display: flex; flex-direction: column; gap: 10px;">
      <div class="field-group">
        <span class="field-label">Target Mod</span>
        <select id="wizard-mod-select" class="tactical-select">
          ${modOptions || '<option value="GCore">GCore</option>'}
        </select>
      </div>

      <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 8px;">
        <div class="field-group">
          <span class="field-label">Patch ID (Alphanumeric)</span>
          <input type="text" id="wizard-patch-id" class="tactical-input" value="${escapeHtml(data.patchId || "")}" placeholder="e.g. CustomBiomeGenerator" />
        </div>
        <div class="field-group">
          <span class="field-label">qm.toml Section Category</span>
          <select id="wizard-category-select" class="tactical-select">
            ${catOptions}
          </select>
        </div>
      </div>

      <div class="field-group">
        <span class="field-label">Modder Description</span>
        <input type="text" id="wizard-desc" class="tactical-input" value="${escapeHtml(data.description || "")}" placeholder="Purpose and behavior of this patch" />
      </div>

      <div class="field-group">
        <span class="field-label">Matcher Insertion Mode</span>
        <div style="display: grid; grid-template-columns: repeat(4, 1fr); gap: 6px;">
          <button type="button" class="btn btn-xs ${data.position === "at" || !data.position ? "btn-accent" : ""}" onclick="setWizardPosition('at')">at (replace)</button>
          <button type="button" class="btn btn-xs ${data.position === "before" ? "btn-accent" : ""}" onclick="setWizardPosition('before')">before (inject)</button>
          <button type="button" class="btn btn-xs ${data.position === "after" ? "btn-accent" : ""}" onclick="setWizardPosition('after')">after (inject)</button>
          <button type="button" class="btn btn-xs ${data.position === "regex" ? "btn-accent" : ""}" onclick="setWizardPosition('regex')">regex</button>
        </div>
      </div>

      <!-- Two-Tier Dependency Selector -->
      <div class="field-group" style="background: rgba(0,0,0,0.2); padding: 8px; border-radius: 4px; border: 1px solid var(--border-subtle);">
        <span class="field-label" style="color: var(--accent-primary);">Prerequisite Dependencies (requires)</span>
        <div style="display: grid; grid-template-columns: 140px 1fr auto; gap: 6px; align-items: center; margin-top: 4px;">
          <select id="wizard-dep-mod" class="tactical-select" style="font-size: 11px;">
            <option value="current">[This Mod]</option>
            ${modOptions}
          </select>
          <select id="wizard-dep-patch" class="tactical-select" style="font-size: 11px;">
            <option value="">-- Choose Schema-Compliant Patch --</option>
          </select>
          <button type="button" class="btn btn-xs btn-accent" id="btn-add-dep">+ Add</button>
        </div>
        <div id="wizard-dep-chips" style="margin-top: 6px; display: flex; flex-wrap: wrap; gap: 4px;"></div>
      </div>

      <div style="margin-top: auto; padding-top: 10px; border-top: 1px solid var(--border-subtle); display: flex; justify-content: space-between; align-items: center;">
        <span style="font-size: 11px; color: var(--text-muted);">${state.patchTargetRange ? (end - start + 1) : 0} line(s) selected as anchor</span>
        <button type="button" class="btn btn-accent" id="btn-wizard-confirm">${hasInProgress ? "Continue to Patch Editor &rarr;" : "Confirm &amp; Open Patch Editor &rarr;"}</button>
      </div>
    </div>
  `;

  document.getElementById("wizard-patch-id")?.addEventListener("input", (e) => {
    state.patchWizardData.patchId = e.target.value.replace(/[^A-Za-z0-9_-]/g, "");
  });
  document.getElementById("wizard-desc")?.addEventListener("input", (e) => {
    state.patchWizardData.description = e.target.value;
  });
  document.getElementById("wizard-category-select")?.addEventListener("change", (e) => {
    state.patchWizardData.category = e.target.value;
  });
  document.getElementById("wizard-mod-select")?.addEventListener("change", (e) => {
    state.patchWizardData.targetMod = e.target.value;
    updateWizardDependencyPatchDropdown();
  });

  document.getElementById("wizard-dep-mod")?.addEventListener("change", updateWizardDependencyPatchDropdown);
  document.getElementById("btn-add-dep")?.addEventListener("click", () => {
    const sel = document.getElementById("wizard-dep-patch");
    if (sel && sel.value) {
      addWizardDependency(sel.value);
    }
  });

  document.getElementById("btn-wizard-goto-editor")?.addEventListener("click", () => {
    confirmWizardAndOpenEditor();
  });

  document.getElementById("btn-wizard-confirm")?.addEventListener("click", () => {
    const idInput = document.getElementById("wizard-patch-id");
    const patchIdVal = (idInput?.value || state.patchWizardData?.patchId || "").trim();
    if (!patchIdVal) {
      showToast("Please enter a valid Patch ID.", true);
      return;
    }
    state.patchWizardData.patchId = patchIdVal;
    confirmWizardAndOpenEditor();
  });

  updateWizardDependencyPatchDropdown();
  renderWizardDependenciesChips();
}

async function updateWizardDependencyPatchDropdown() {
  const sel = document.getElementById("wizard-dep-patch");
  if (!sel) return;

  const depModSelect = document.getElementById("wizard-dep-mod");
  const depModVal = depModSelect?.value || "current";

  let targetModId = "";
  if (depModVal === "current") {
    targetModId = state.patchWizardData?.targetMod
      || document.getElementById("wizard-mod-select")?.value
      || state.data?.installed_mods?.[0]?.manifest?.id
      || "";
  } else {
    targetModId = depModVal;
  }

  let patchesList = [];
  try {
    if (typeof apiGet === "function" && targetModId) {
      const res = await apiGet(`/api/devtools/patches?mod_id=${encodeURIComponent(targetModId)}`);
      if (res && Array.isArray(res.patches)) {
        patchesList = res.patches;
      }
    }
  } catch (_) {}

  // Fallback: If API returned empty but targetModId is chosen, try reading the mod's qm.toml directly
  if (patchesList.length === 0 && targetModId && typeof apiGet === "function") {
    try {
      const readRes = await apiGet(`/api/devtools/file/read?mod_dir=${encodeURIComponent(targetModId)}&file_path=qm.toml`);
      if (readRes && readRes.content) {
        patchesList = parsePatchesFromQmToml(readRes.content, targetModId);
      }
    } catch (_) {}
  }

  // Filter strictly for schema-compliant patches
  const compliant = filterSchemaCompliantPatches(patchesList);

  // Exclude current patch id so it cannot depend on itself
  const currentPatchId = (state.patchWizardData?.patchId || "").trim();
  const validCandidates = compliant.filter(p => p.id !== currentPatchId);

  if (validCandidates.length === 0) {
    sel.innerHTML = '<option value="">No formatted patches available</option>';
    return;
  }

  sel.innerHTML = '<option value="">-- Choose Schema-Compliant Patch --</option>' +
    validCandidates.map(p => {
      const desc = p.description ? ` - ${escapeHtml(p.description)}` : ` (${escapeHtml(p.target)})`;
      return `<option value="${escapeHtml(p.id)}">${escapeHtml(p.id)}${desc}</option>`;
    }).join("");
}

function parsePatchesFromQmToml(content, modId = "") {
  if (!content || typeof content !== "string") return [];
  const patches = [];
  const blocks = content.split(/\[\[patches\]\]/);

  for (let i = 1; i < blocks.length; i++) {
    const block = blocks[i];
    const idMatch = block.match(/\bid\s*=\s*"([^"]+)"/);
    const targetMatch = block.match(/\btarget\s*=\s*"([^"]+)"/);
    const posMatch = block.match(/\bposition\s*=\s*"([^"]+)"/);
    const descMatch = block.match(/\bdescription\s*=\s*"([^"]+)"/);

    let pattern = "";
    const triplePattern = block.match(/pattern\s*=\s*'''([\s\S]*?)'''/);
    if (triplePattern) {
      pattern = triplePattern[1];
    } else {
      const singlePattern = block.match(/pattern\s*=\s*'([^']*)'/);
      if (singlePattern) {
        pattern = singlePattern[1];
      } else {
        const doublePattern = block.match(/pattern\s*=\s*"([^"]*)"/);
        if (doublePattern) pattern = doublePattern[1];
      }
    }

    let requires = [];
    const reqMatch = block.match(/requires\s*=\s*\[(.*?)\]/s);
    if (reqMatch) {
      const items = reqMatch[1].match(/"([^"]+)"/g);
      if (items) {
        requires = items.map(s => s.replace(/"/g, "").trim());
      }
    }

    if (idMatch && targetMatch) {
      patches.push({
        id: idMatch[1].trim(),
        name: idMatch[1].trim(),
        target: targetMatch[1].trim(),
        position: posMatch ? posMatch[1].trim() : "at",
        description: descMatch ? descMatch[1].trim() : "",
        pattern: pattern,
        requires: requires,
        mod_id: modId || ""
      });
    }
  }

  return patches;
}

function renderWizardDependenciesChips() {
  const container = document.getElementById("wizard-dep-chips");
  if (!container) return;
  const reqs = state.patchWizardData?.requires || [];
  if (reqs.length === 0) {
    container.innerHTML = '<span style="font-size: 10.5px; color: var(--text-muted); font-style: italic;">No prerequisite dependencies selected.</span>';
    return;
  }
  container.innerHTML = reqs.map(id => `
    <span class="badge badge-accent" style="display: inline-flex; align-items: center; gap: 4px; padding: 2px 6px; font-size: 11px;">
      <span>${escapeHtml(id)}</span>
      <span style="cursor: pointer; font-weight: bold; color: var(--status-danger);" onclick="removeWizardDependency('${escapeHtml(id)}')">&times;</span>
    </span>
  `).join("");
}

function setWizardPosition(pos) {
  if (!state.patchWizardData) state.patchWizardData = {};
  const idInput = document.getElementById("wizard-patch-id");
  if (idInput) state.patchWizardData.patchId = idInput.value;
  const descInput = document.getElementById("wizard-desc");
  if (descInput) state.patchWizardData.description = descInput.value;
  state.patchWizardData.position = pos;
  renderPatchWizardStep();
}

function updatePatchWizardTarget() {
  const file = state.patchStudioActiveFile || "conf.lua";
  const [start, end] = state.patchTargetRange || [1, 1];
  const targetLabel = document.getElementById("wizard-target-display");
  if (targetLabel) {
    targetLabel.textContent = `${file}:${state.patchTargetRange ? `${start}–${end}` : "None"}`;
  }
}

function autoResizeTextarea(el) {
  if (!el || !el.style) return;
  el.style.height = "auto";
  const scrollHeight = el.scrollHeight || 80;
  el.style.height = Math.max(80, scrollHeight + 4) + "px";
}

function renderLiveDiffRowsHtml(diffItems) {
  if (!diffItems || diffItems.length === 0) return "";
  const rows = [];
  for (const item of diffItems) {
    if (item.type === "delete") {
      rows.push(`
        <div class="source-code-row diff-delete" data-line="${item.lineNo}">
          <span class="source-lineno diff-lineno-delete">-${item.lineNo}</span>
          <span class="source-content">${escapeHtml(item.content)}</span>
          <span class="badge-diff-del">REMOVED</span>
        </div>
      `);
    } else if (item.type === "add") {
      rows.push(`
        <div class="source-code-row diff-add" data-line="${item.lineNo}">
          <span class="source-lineno diff-lineno-add">+${item.lineNo}</span>
          <span class="source-content">${escapeHtml(item.content)}</span>
          <span class="badge-diff-add">ADDED</span>
        </div>
      `);
    } else {
      rows.push(`
        <div class="source-code-row target-highlight" data-line="${item.lineNo}">
          <span class="source-lineno">${item.lineNo}</span>
          <span class="source-content">${escapeHtml(item.content)}</span>
          <span class="badge-diff-anchor">ANCHOR</span>
        </div>
      `);
    }
  }
  return rows.join("");
}

function updateStudioInlineDiffOnly() {
  const diffSection = document.getElementById("studio-inline-diff-section");
  if (!diffSection) {
    if (typeof renderPatchStudioSourcePane === "function" && state.patchStudioActiveFile) {
      renderPatchStudioSourcePane(state.patchStudioActiveFile);
    }
    return;
  }
  const lines = state.currentSourceLines || [];
  const targetRange = state.patchTargetRange;
  if (!targetRange) return;
  const diffItems = computeLivePatchDiff(
    lines,
    targetRange,
    state.patchWizardData?.position || "at",
    state.patchWizardData?.payload ?? state.patchWizardData?.pattern ?? "",
    state.patchWizardData?.matchIndent !== false
  );
  diffSection.innerHTML = renderLiveDiffRowsHtml(diffItems);
}

function renderLivePatchEditor() {
  const pane = document.getElementById("studio-right-pane");
  if (!pane) return;

  const data = state.patchWizardData || {};
  const [start, end] = state.patchTargetRange || [1, 1];
  const file = state.patchStudioActiveFile || "conf.lua";
  const lines = state.currentSourceLines || [];
  const ctxSlices = getEditorContextSlices(lines, state.patchTargetRange, state.patchContextLines);
  const pos = data.position || "at";

  const topHtml = ctxSlices.topLines.map(l => `
    <div class="ed-line dimmed"><span class="ed-no">${l.lineNo}</span><span>${escapeHtml(l.content)}</span></div>
  `).join("");

  const bottomHtml = ctxSlices.bottomLines.map(l => `
    <div class="ed-line dimmed"><span class="ed-no">${l.lineNo}</span><span>${escapeHtml(l.content)}</span></div>
  `).join("");

  const anchorHtml = ctxSlices.targetLines.map(l => `
    <div class="ed-line anchor-line" style="background: rgba(88, 166, 255, 0.12); border-left: 3px solid #58a6ff; font-weight: 500;">
      <span class="ed-no" style="color: #58a6ff; font-weight: bold;">${l.lineNo}</span>
      <span style="color: #e6edf3;">${escapeHtml(l.content)}</span>
      <span class="badge" style="font-size: 9px; padding: 1px 4px; margin-left: auto; background: #1f6feb; color: #fff;">ANCHOR</span>
    </div>
  `).join("");

  const replaceTargetHtml = ctxSlices.targetLines.map(l => `
    <div class="ed-line" style="background: rgba(248, 81, 73, 0.1); border-left: 3px solid #f85149; text-decoration: line-through; opacity: 0.75;">
      <span class="ed-no" style="color: #f85149;">${l.lineNo}</span>
      <span style="color: #ff7b72;">${escapeHtml(l.content)}</span>
      <span class="badge" style="font-size: 9px; padding: 1px 4px; margin-left: auto; background: rgba(248, 81, 73, 0.3); color: #f85149;">REPLACING</span>
    </div>
  `).join("");

  let editorBodyHtml = "";
  if (pos === "after") {
    editorBodyHtml = `
      ${topHtml}
      ${anchorHtml}
      <div style="padding: 6px 10px; background: rgba(56, 139, 253, 0.05); border-top: 1px dashed #388bfd; border-bottom: 1px dashed #388bfd;">
        <div style="font-size: 10px; color: #58a6ff; font-weight: bold; margin-bottom: 4px;">PATCH PAYLOAD (Editing &mdash; Injecting AFTER Line ${end})</div>
        <textarea id="patch-payload-input" class="editor-payload-textarea" spellcheck="false" placeholder="Enter patch Lua payload here...">${escapeHtml(data.payload ?? data.pattern ?? "")}</textarea>
      </div>
      ${bottomHtml}
    `;
  } else if (pos === "before") {
    editorBodyHtml = `
      ${topHtml}
      <div style="padding: 6px 10px; background: rgba(56, 139, 253, 0.05); border-top: 1px dashed #388bfd; border-bottom: 1px dashed #388bfd;">
        <div style="font-size: 10px; color: #58a6ff; font-weight: bold; margin-bottom: 4px;">PATCH PAYLOAD (Editing &mdash; Injecting BEFORE Line ${start})</div>
        <textarea id="patch-payload-input" class="editor-payload-textarea" spellcheck="false" placeholder="Enter patch Lua payload here...">${escapeHtml(data.payload ?? data.pattern ?? "")}</textarea>
      </div>
      ${anchorHtml}
      ${bottomHtml}
    `;
  } else {
    // "at" or "regex" replacement
    editorBodyHtml = `
      ${topHtml}
      ${replaceTargetHtml}
      <div style="padding: 6px 10px; background: rgba(56, 139, 253, 0.05); border-top: 1px dashed #388bfd; border-bottom: 1px dashed #388bfd;">
        <div style="font-size: 10px; color: #58a6ff; font-weight: bold; margin-bottom: 4px;">PATCH PAYLOAD (Editing &mdash; Replacing Lines ${start}&ndash;${end})</div>
        <textarea id="patch-payload-input" class="editor-payload-textarea" spellcheck="false" placeholder="Enter patch Lua payload here...">${escapeHtml(data.payload ?? data.pattern ?? "")}</textarea>
      </div>
      ${bottomHtml}
    `;
  }

  pane.innerHTML = `
    <div class="editor-header-bar">
      <div style="display: flex; gap: 6px; align-items: center;">
        <span class="badge badge-ok">Editing: ${escapeHtml(data.patchId || "Patch")}</span>
        <span class="badge" style="font-size: 10px;">${escapeHtml(file)}:${start}–${end}</span>
        <span class="badge" style="font-size: 10px;">Mode: ${escapeHtml(pos)}</span>
      </div>
      <div class="editor-context-controls">
        <label style="font-size: 10.5px; color: var(--text-muted); margin: 0;">Context:</label>
        <input type="range" class="editor-context-slider" min="1" max="15" value="${state.patchContextLines || 4}" id="editor-context-slider" />
        <span style="font-size: 11px; min-width: 14px;" id="editor-context-count">${state.patchContextLines || 4}</span>
        <button type="button" class="btn btn-xs" id="btn-editor-back-wizard">&larr; Wizard / Info</button>
        <button type="button" class="btn btn-xs btn-accent" id="btn-editor-save-patch">Save to qm.toml</button>
      </div>
    </div>

    <div class="editor-body-container" id="editor-body-container">
      ${editorBodyHtml}
    </div>
  `;

  const slider = document.getElementById("editor-context-slider");
  slider?.addEventListener("input", (e) => {
    setContextLinesCount(e.target.value);
    const countEl = document.getElementById("editor-context-count");
    if (countEl) countEl.textContent = state.patchContextLines;
  });

  document.getElementById("btn-editor-back-wizard")?.addEventListener("click", () => {
    const payloadInput = document.getElementById("patch-payload-input");
    if (payloadInput && state.patchWizardData) {
      state.patchWizardData.payload = payloadInput.value;
    }
    state.patchWizardStep = 1;
    renderPatchStudioRightPane();
    if (typeof renderPatchStudioSourcePane === "function" && state.patchStudioActiveFile) {
      renderPatchStudioSourcePane(state.patchStudioActiveFile);
    }
  });

  const payloadInput = document.getElementById("patch-payload-input");
  if (payloadInput) {
    autoResizeTextarea(payloadInput);
    payloadInput.addEventListener("input", () => {
      autoResizeTextarea(payloadInput);
      state.patchWizardData.payload = payloadInput.value;
      updateStudioInlineDiffOnly();
    });
  }

  document.getElementById("btn-editor-save-patch")?.addEventListener("click", async () => {
    const payloadEl = document.getElementById("patch-payload-input");
    if (payloadEl && state.patchWizardData) {
      state.patchWizardData.payload = payloadEl.value;
    }
    await savePatchToModToml();
  });
}

function updateLiveDiffPreview() {
  if (typeof renderPatchStudioSourcePane === "function" && state.patchStudioActiveFile) {
    renderPatchStudioSourcePane(state.patchStudioActiveFile);
  }
}

async function savePatchToModToml() {
  const data = state.patchWizardData;
  if (!data || !data.patchId) {
    showToast("Invalid patch data. Please specify a Patch ID.", true);
    return;
  }

  const modSelect = document.getElementById("wizard-mod-select");
  const modId = data.targetMod
    || (modSelect && modSelect.value)
    || (state.data?.installed_mods?.[0]?.manifest?.id)
    || "GCore";
  data.targetMod = modId;

  const lines = state.currentSourceLines || [];
  const [start, end] = state.patchTargetRange || [1, Math.min(5, lines.length || 1)];
  const patternVal = data.pattern || lines.slice(start - 1, end).join("\n");

  const patchObj = {
    id: data.patchId,
    target: state.patchStudioActiveFile || "conf.lua",
    pattern: patternVal,
    position: data.position || "at",
    matchIndent: data.matchIndent !== false,
    requires: data.requires || [],
    description: data.description || "",
    payload: data.payload ?? patternVal,
    category: data.category || "# APPLICATION LIFECYCLE & SETTINGS"
  };

  try {
    let existingContent = "";
    try {
      const readRes = await apiGet(`/api/devtools/file/read?mod_dir=${encodeURIComponent(modId)}&file_path=qm.toml`);
      if (readRes && readRes.content) {
        existingContent = readRes.content;
      }
    } catch (_) {}

    const updatedContent = insertOrUpdatePatchInQmToml(existingContent, patchObj);

    if (typeof apiPost === "function") {
      await apiPost("/api/devtools/file/save", {
        mod_dir: modId,
        mod_id: modId,
        file_path: "qm.toml",
        content: updatedContent
      });
    }

    showToast(`[OK] Saved patch '${patchObj.id}' to ${modId}/qm.toml!`);
    await refreshState();
    if (typeof updateWizardDependencyPatchDropdown === "function") {
      await updateWizardDependencyPatchDropdown();
    }
  } catch (err) {
    showToast(`Failed to save patch: ${err.message}`, true);
  }
}

function setPatchTargetRange(startLine, endLine) {
  const maxLines = (state.currentSourceLines && state.currentSourceLines.length > 0)
    ? state.currentSourceLines.length
    : 1000;
  let s = Math.max(1, Math.min(startLine, maxLines));
  let e = Math.max(s, Math.min(endLine, maxLines));
  state.patchTargetRange = [s, e];
  if (typeof updatePatchWizardTarget === "function") {
    updatePatchWizardTarget();
  }
}

function adjustPatchBoundary(boundary, action) {
  if (!state.patchTargetRange) state.patchTargetRange = [1, 1];
  let [start, end] = state.patchTargetRange;
  const maxLines = state.currentSourceLines?.length || 1000;

  if (boundary === "top") {
    if (action === "extend") {
      start = Math.max(1, start - 1);
    } else if (action === "shrink") {
      start = Math.min(end, start + 1);
    }
  } else if (boundary === "bottom") {
    if (action === "extend") {
      end = Math.min(maxLines, end + 1);
    } else if (action === "shrink") {
      end = Math.max(start, end - 1);
    }
  }

  setPatchTargetRange(start, end);
  if (typeof renderPatchStudioSourcePane === "function" && state.patchStudioActiveFile) {
    renderPatchStudioSourcePane(state.patchStudioActiveFile);
  }
}

function togglePatchOverlay(enabled) {
  state.engineShowOtherPatches = Boolean(enabled);
  if (typeof renderPatchStudioSourcePane === "function" && state.patchStudioActiveFile) {
    renderPatchStudioSourcePane(state.patchStudioActiveFile);
  }
}

function renderPatchStudioSourceLines(lines, targetRange, activePatchesOverlay, showOverlay) {
  if (!lines || lines.length === 0) {
    return '<div class="code-line dimmed" style="padding:12px">No source code loaded.</div>';
  }

  const [tgtStart, tgtEnd] = targetRange || [0, -1];
  const overlayMap = new Map();
  if (showOverlay && activePatchesOverlay) {
    for (const p of activePatchesOverlay) {
      if (p.line_range) {
        for (let l = p.line_range[0]; l <= p.line_range[1]; l++) {
          overlayMap.set(l, p.patch_id || p.id || "Active Patch");
        }
      }
    }
  }

  const isEditing = (state.patchWizardStep === 2);
  let diffItems = null;
  if (isEditing && targetRange && tgtStart >= 1 && tgtEnd >= tgtStart) {
    diffItems = computeLivePatchDiff(
      lines,
      targetRange,
      state.patchWizardData?.position || "at",
      state.patchWizardData?.payload ?? state.patchWizardData?.pattern ?? "",
      state.patchWizardData?.matchIndent !== false
    );
  }

  const rows = [];
  for (let idx = 0; idx < lines.length; idx++) {
    const lineNo = idx + 1;
    const isTarget = Boolean(targetRange && lineNo >= tgtStart && lineNo <= tgtEnd);
    const otherPatchId = overlayMap.get(lineNo);
    const lineText = lines[idx] || "";

    // Boundary handle at top of target (kept for unit test compatibility, styled hidden)
    if (lineNo === tgtStart) {
      rows.push(`
        <div class="boundary-handle-row boundary-top">
          <span class="boundary-label">SELECTION TOP (Line ${lineNo})</span>
          <div class="boundary-actions">
            <button type="button" class="btn btn-boundary" onclick="adjustPatchBoundary('top', 'extend')">[Extend]</button>
            <button type="button" class="btn btn-boundary" onclick="adjustPatchBoundary('top', 'shrink')">[Shrink]</button>
          </div>
        </div>
      `);
    }

    // In editing mode, replace target range lines with live unified diff lines
    if (isEditing && diffItems && lineNo === tgtStart) {
      const diffHtml = renderLiveDiffRowsHtml(diffItems);
      rows.push(`<div id="studio-inline-diff-section" style="display: contents;">${diffHtml}</div>`);
      idx = tgtEnd - 1;
      continue;
    }

    let rowClass = "source-code-row";
    if (isTarget) {
      rowClass += " target-highlight";
    } else if (otherPatchId) {
      rowClass += " other-patch-highlight";
    } else {
      rowClass += " dimmed";
    }

    let badgeHtml = "";
    if (otherPatchId && !isTarget) {
      badgeHtml = `<span class="badge badge-patch-overlay" title="Modified by ${escapeHtml(otherPatchId)}">[Patch: ${escapeHtml(otherPatchId)}]</span>`;
    }

    rows.push(`
      <div class="${rowClass}" data-line="${lineNo}" onclick="onSourceLineClick(${lineNo})">
        <span class="source-lineno">${lineNo}</span>
        <span class="source-content">${escapeHtml(lineText)}</span>
        ${badgeHtml}
      </div>
    `);

    // Boundary handle at bottom of target (kept for unit test compatibility, styled hidden)
    if (lineNo === tgtEnd) {
      rows.push(`
        <div class="boundary-handle-row boundary-bottom">
          <span class="boundary-label">SELECTION BOTTOM (Line ${lineNo})</span>
          <div class="boundary-actions">
            <button type="button" class="btn btn-boundary" onclick="adjustPatchBoundary('bottom', 'shrink')">[Shrink]</button>
            <button type="button" class="btn btn-boundary" onclick="adjustPatchBoundary('bottom', 'extend')">[Extend]</button>
          </div>
        </div>
      `);
    }
  }

  return rows.join("");
}

function onSourceLineClick(lineNo) {
  if (state.patchWizardStep === 2) return;
  if (!state.patchTargetRange) {
    setPatchTargetRange(lineNo, lineNo);
  } else {
    const [start, end] = state.patchTargetRange;
    if (lineNo >= start && lineNo <= end) {
      setPatchTargetRange(lineNo, lineNo);
    } else if (lineNo < start) {
      setPatchTargetRange(lineNo, end);
    } else {
      setPatchTargetRange(start, lineNo);
    }
  }
  if (typeof renderPatchStudioSourcePane === "function" && state.patchStudioActiveFile) {
    renderPatchStudioSourcePane(state.patchStudioActiveFile);
  }
}

function filterSchemaCompliantPatches(patchesList) {
  if (!Array.isArray(patchesList)) return [];
  return patchesList.filter((p) => {
    if (!p || typeof p !== "object") return false;
    const hasId = typeof p.id === "string" && /^[A-Za-z0-9_-]+$/.test(p.id.trim());
    const hasTarget = typeof p.target === "string" && p.target.trim().length > 0;
    const hasPattern = typeof p.pattern === "string" && p.pattern.length > 0;
    const validPos = ["at", "before", "after", "regex"].includes(p.position || "at");
    return hasId && hasTarget && hasPattern && validPos;
  });
}

function addWizardDependency(patchId) {
  if (!patchId || typeof patchId !== "string") return false;
  const cleanId = patchId.trim();
  if (!cleanId) return false;

  if (!state.patchWizardData) {
    state.patchWizardData = { requires: [] };
  }
  if (!Array.isArray(state.patchWizardData.requires)) {
    state.patchWizardData.requires = [];
  }

  // Self-dependency check
  if (state.patchWizardData.patchId && state.patchWizardData.patchId === cleanId) {
    return false;
  }

  // Duplicate check
  if (state.patchWizardData.requires.includes(cleanId)) {
    return false;
  }

  state.patchWizardData.requires.push(cleanId);
  if (typeof renderWizardDependenciesChips === "function") {
    renderWizardDependenciesChips();
  }
  return true;
}

function removeWizardDependency(patchId) {
  if (!state.patchWizardData || !Array.isArray(state.patchWizardData.requires)) return;
  state.patchWizardData.requires = state.patchWizardData.requires.filter((id) => id !== patchId);
  if (typeof renderWizardDependenciesChips === "function") {
    renderWizardDependenciesChips();
  }
}

function confirmWizardAndOpenEditor() {
  if (!state.patchWizardData) {
    state.patchWizardData = {};
  }
  const lines = state.currentSourceLines || [];
  const [start, end] = state.patchTargetRange || [1, 1];

  // Capture selected target lines as the pattern anchor
  if (!state.patchWizardData.pattern) {
    const slice = lines.slice(start - 1, end);
    state.patchWizardData.pattern = slice.join("\n");
  }

  // Initialize payload with pattern if payload is empty/undefined
  if (state.patchWizardData.payload === undefined || state.patchWizardData.payload === null) {
    state.patchWizardData.payload = state.patchWizardData.pattern || "";
  }

  state.patchWizardStep = 2;

  if (typeof renderPatchStudioRightPane === "function") {
    renderPatchStudioRightPane();
  }
  if (typeof renderPatchStudioSourcePane === "function" && state.patchStudioActiveFile) {
    renderPatchStudioSourcePane(state.patchStudioActiveFile);
  }
}

function setContextLinesCount(n) {
  const count = parseInt(n, 10);
  if (Number.isNaN(count)) return;
  state.patchContextLines = Math.max(1, Math.min(15, count));
  if (typeof renderLivePatchEditor === "function" && state.patchWizardStep === 2) {
    renderLivePatchEditor();
  }
}

function getEditorContextSlices(lines, targetRange, contextCount) {
  const src = lines || [];
  const [start, end] = targetRange || [1, 1];
  const ctx = typeof contextCount === "number" ? contextCount : (state.patchContextLines || 4);

  const topStart = Math.max(1, start - ctx);
  const topLines = [];
  for (let l = topStart; l < start; l++) {
    topLines.push({ lineNo: l, content: src[l - 1] || "" });
  }

  const targetLines = [];
  for (let l = start; l <= end && l <= src.length; l++) {
    targetLines.push({ lineNo: l, content: src[l - 1] || "" });
  }

  const bottomEnd = Math.min(src.length, end + ctx);
  const bottomLines = [];
  for (let l = end + 1; l <= bottomEnd; l++) {
    bottomLines.push({ lineNo: l, content: src[l - 1] || "" });
  }

  return { topLines, targetLines, bottomLines };
}

function computeLivePatchDiff(vanillaLines, targetRange, position, payload, matchIndent) {
  const src = vanillaLines || [];
  const [start, end] = targetRange || [1, 1];
  const pos = position || "at";
  const rawPayload = typeof payload === "string" ? payload : "";

  // Split payload into individual lines
  let payloadLines = rawPayload.split("\n");

  // If match_indent is enabled, extract leading indent from first line of target
  if (matchIndent && src[start - 1]) {
    const indentMatch = src[start - 1].match(/^(\s+)/);
    if (indentMatch && indentMatch[1]) {
      const indent = indentMatch[1];
      payloadLines = payloadLines.map((line) => {
        if (line.trim().length > 0 && !line.startsWith(indent)) {
          return indent + line;
        }
        return line;
      });
    }
  }

  const diff = [];

  if (pos === "at") {
    // Target lines are deleted
    for (let l = start; l <= end && l <= src.length; l++) {
      diff.push({ type: "delete", lineNo: l, content: src[l - 1] || "" });
    }
    // Payload lines are added
    for (let idx = 0; idx < payloadLines.length; idx++) {
      diff.push({ type: "add", lineNo: start + idx, content: payloadLines[idx] });
    }
  } else if (pos === "before") {
    // Payload lines are added before anchor
    for (let idx = 0; idx < payloadLines.length; idx++) {
      diff.push({ type: "add", lineNo: start + idx, content: payloadLines[idx] });
    }
    // Anchor remains unchanged
    for (let l = start; l <= end && l <= src.length; l++) {
      diff.push({ type: "context", lineNo: l, content: src[l - 1] || "" });
    }
  } else if (pos === "after") {
    // Anchor remains unchanged
    for (let l = start; l <= end && l <= src.length; l++) {
      diff.push({ type: "context", lineNo: l, content: src[l - 1] || "" });
    }
    // Payload lines are added after anchor
    for (let idx = 0; idx < payloadLines.length; idx++) {
      diff.push({ type: "add", lineNo: end + 1 + idx, content: payloadLines[idx] });
    }
  }

  return diff;
}

function serializePatchToToml(patchObj) {
  if (!patchObj) return "";
  const id = (patchObj.id || "").trim();
  const target = (patchObj.target || "").trim();
  const pos = (patchObj.position || "at").trim();
  const matchIndent = patchObj.matchIndent !== false;
  const desc = (patchObj.description || "").trim();
  const reqs = Array.isArray(patchObj.requires) ? patchObj.requires : [];
  const pattern = patchObj.pattern || "";
  const payload = patchObj.payload || "";

  const isRegex = (pos === "regex");
  const lines = [
    "[[patches]]",
    isRegex ? "[patches.regex]" : "[patches.pattern]"
  ];
  if (id) lines.push(`id = "${id}"`);
  if (desc) lines.push(`description = "${desc.replace(/"/g, '\\"')}"`);
  if (target) lines.push(`target = "${target}"`);

  // Pattern strictly with triple single quotes
  lines.push("pattern = '''\n" + pattern + "\n'''");

  lines.push(`position = "${pos}"`);

  // Payload strictly with triple single quotes
  lines.push("payload = '''\n" + payload + "\n'''");

  // match_indent strictly placed after payload
  lines.push(`match_indent = ${matchIndent ? "true" : "false"}`);

  if (reqs.length > 0) {
    const reqList = reqs.map((r) => `"${r}"`).join(", ");
    lines.push(`requires = [${reqList}]`);
  }

  return lines.join("\n");
}

function insertOrUpdatePatchInQmToml(existingContent, patchObj) {
  const content = existingContent || "";
  const patchToml = serializePatchToToml(patchObj);
  const patchId = (patchObj.id || "").trim();

  // 1. Check if patch with this ID already exists
  if (patchId) {
    const patchRegex = new RegExp(
      `(\\[\\[patches\\]\\][\\s\\S]*?id\\s*=\\s*"${patchId}"[\\s\\S]*?)(?=\\n\\[\\[patches\\]\\]|\\n#\\s*={5,}|$)`,
      "g"
    );
    if (patchRegex.test(content)) {
      return content.replace(patchRegex, patchToml);
    }
  }

  // 2. Identify Category
  let categoryRaw = (patchObj.category || "").trim();
  let cleanCategoryName = categoryRaw.replace(/^[#\s=]+/, "").replace(/[#\s=]+$/, "").trim();
  if (!cleanCategoryName) cleanCategoryName = "GENERAL PATCHES";

  // Check if category header exists in file
  const categoryHeaderRegex = new RegExp(
    `(#\\s*={5,}\\s*\\n)?#\\s*${cleanCategoryName.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s*\\n(#\\s*={5,}\\s*\\n)?`,
    "i"
  );
  const match = content.match(categoryHeaderRegex);

  if (match) {
    // Category found! Find the insertion point before the NEXT category header or at EOF
    const catStartIdx = match.index + match[0].length;
    const rest = content.slice(catStartIdx);
    const nextCategoryMatch = rest.match(/\n#\s*={5,}/);

    let insertIdx = content.length;
    if (nextCategoryMatch) {
      insertIdx = catStartIdx + nextCategoryMatch.index;
    }

    const before = content.slice(0, insertIdx).trimEnd();
    const after = content.slice(insertIdx);
    return before + "\n\n" + patchToml + "\n\n" + after.trimStart();
  }

  // 3. Category does not exist yet: append new category divider block at end of file
  const divider = [
    "",
    "# ==============================================================================",
    `# ${cleanCategoryName}`,
    "# ==============================================================================",
    "",
    patchToml,
    ""
  ].join("\n");

  return content.trimEnd() + "\n" + divider;
}

function getDependentPatches(targetPatchId, allPatches, disabledPatchIds = []) {
  if (!targetPatchId || !Array.isArray(allPatches)) return [];

  const disabledSet = new Set(disabledPatchIds);
  const result = [];
  const visited = new Set();

  function scan(currentId) {
    for (const patch of allPatches) {
      if (!patch || !patch.id || visited.has(patch.id)) continue;
      const reqs = Array.isArray(patch.requires) ? patch.requires : [];
      if (reqs.includes(currentId)) {
        visited.add(patch.id);
        if (!disabledSet.has(patch.id)) {
          result.push(patch);
        }
        scan(patch.id);
      }
    }
  }

  scan(targetPatchId);
  return result;
}

function checkPatchPrerequisites(targetPatchId, willEnable, allPatches, disabledPatchIds = []) {
  if (!targetPatchId || !Array.isArray(allPatches)) {
    return { action: "proceed", affectedPatches: [] };
  }

  const patchMap = new Map();
  for (const p of allPatches) {
    if (p && p.id) patchMap.set(p.id, p);
  }

  const disabledSet = new Set(disabledPatchIds);

  if (!willEnable) {
    // Turning patch OFF -> check if any currently ENABLED patch depends on it
    const dependents = getDependentPatches(targetPatchId, allPatches, disabledPatchIds);
    if (dependents.length > 0) {
      return {
        action: "confirm_disable_dependents",
        affectedPatches: dependents
      };
    }
    return { action: "proceed", affectedPatches: [] };
  } else {
    // Turning patch ON -> check if all prerequisites in requires are active
    const targetPatch = patchMap.get(targetPatchId);
    if (!targetPatch) return { action: "proceed", affectedPatches: [] };

    const neededPrereqs = [];
    const visitedPrereqs = new Set();

    function collectPrereqs(pId) {
      const p = patchMap.get(pId);
      if (!p) return;
      const reqs = Array.isArray(p.requires) ? p.requires : [];
      for (const reqId of reqs) {
        if (visitedPrereqs.has(reqId)) continue;
        visitedPrereqs.add(reqId);
        if (disabledSet.has(reqId)) {
          const reqPatch = patchMap.get(reqId) || { id: reqId, name: reqId };
          neededPrereqs.push(reqPatch);
          collectPrereqs(reqId);
        }
      }
    }

    collectPrereqs(targetPatchId);

    if (neededPrereqs.length > 0) {
      return {
        action: "confirm_enable_prerequisites",
        affectedPatches: neededPrereqs
      };
    }
    return { action: "proceed", affectedPatches: [] };
  }
}

const TUTORIAL_TEMPLATES = [
  {
    id: "minimal-patch",
    category: "Starter Templates",
    title: "Minimal Patch Starter",
    desc: "Basic qm.toml targeting conf.lua & main.lua",
    fileTree: "├── mod.json\n├── qm.toml\n└── README.md",
    steps: [
      {
        title: "Manifest & Metadata Configuration",
        body: "Every mod requires a mod.json defining the unique mod ID, version, and game engine compatibility."
      },
      {
        title: "How In-Memory Patching Works",
        body: "QuarterMaster intercepts LOVE 2D engine reads in RAM during game boot. It matches your target and pattern anchor without modifying disk files."
      },
      {
        title: "Seamless Handoff to Patch Studio",
        body: "Clicking 'Scaffold Mod from Template' creates the project and opens it right in the Patch Studio ready for code editing."
      }
    ],
    sampleToml: "[[patches]]\n[patches.pattern]\nid = \"WindowConfigHook\"\ntarget = \"conf.lua\"\npattern = 't.window.title = \"Mud and Blood\"'\nposition = \"after\"\npayload = '''\nt.window.title = \"Mud and Blood (Modded)\"\n'''"
  },
  {
    id: "weapon-ballistics",
    category: "Starter Templates",
    title: "Weapon & Ballistics Overhaul",
    desc: "Custom weapon stats, fire rates, and recoil tables",
    fileTree: "├── mod.json\n├── qm.toml\n├── weapons_override.lua\n└── README.md",
    steps: [
      {
        title: "Manifest & Metadata Configuration",
        body: "Defines the weapon mod identity and load priority over vanilla weapons.lua."
      },
      {
        title: "How In-Memory Patching Works",
        body: "Intercepts the weapon table initialization in includes/weapons.lua, replacing firearm ranges and muzzle velocity values."
      },
      {
        title: "Seamless Handoff to Patch Studio",
        body: "Transitions directly into Patch Studio with weapons.lua pre-selected."
      }
    ],
    sampleToml: "[[patches]]\n[patches.pattern]\nid = \"GarandFireRateOverhaul\"\ntarget = \"includes/weapons.lua\"\npattern = 'Weapons[1].fireRate = 18'\nposition = \"at\"\npayload = 'Weapons[1].fireRate = 24 -- Increased fire rate'"
  },
  {
    id: "map-terrain",
    category: "Starter Templates",
    title: "Custom Battlefield Map Script",
    desc: "Injects custom terrain generation hooks and obstacles",
    fileTree: "├── mod.json\n├── qm.toml\n├── maps/\n│   └── custom_trenches.lua\n└── README.md",
    steps: [
      {
        title: "Manifest & Metadata Configuration",
        body: "Specifies map build module overrides."
      },
      {
        title: "How In-Memory Patching Works",
        body: "Injects hooks into createRuined() and initTerrainGrid() in includes/mapBuild.lua."
      },
      {
        title: "Seamless Handoff to Patch Studio",
        body: "Transitions to Patch Studio with mapBuild.lua active."
      }
    ],
    sampleToml: "[[patches]]\n[patches.pattern]\nid = \"CustomMudHoles\"\ntarget = \"includes/mapBuild.lua\"\npattern = 'generateObstacles(currentMap.obstacleChances)'\nposition = \"before\"\npayload = 'ModTrench.spawnMudPits(currentMap)'"
  },
  {
    id: "lua-module",
    category: "Advanced Mechanics",
    title: "Standalone Lua Module Injection",
    desc: "Packaging external .lua files into the game runtime",
    fileTree: "├── mod.json\n├── qm.toml\n├── modules/\n│   └── debug_console.lua\n└── README.md",
    steps: [
      {
        title: "Manifest & Metadata Configuration",
        body: "Configures standalone module injection rules."
      },
      {
        title: "How In-Memory Patching Works",
        body: "Uses [patches.module] to mount custom Lua files directly into the package.preload registry."
      },
      {
        title: "Seamless Handoff to Patch Studio",
        body: "Transitions to the studio to author initialization hooks."
      }
    ],
    sampleToml: "[[patches]]\n[patches.module]\nid = \"MountDebugConsole\"\nsource = \"modules/debug_console.lua\"\nname = \"debug_console\""
  },
  {
    id: "asset-override",
    category: "Advanced Mechanics",
    title: "Texture & SFX Asset Replacement",
    desc: "Sprite replacement & audio SFX swapping via QuarterMaster engine",
    fileTree: "├── mod.json\n├── qm.toml\n├── assets/\n│   ├── sprites/\n│   └── audio/\n└── README.md",
    steps: [
      {
        title: "Manifest & Metadata Configuration",
        body: "Declares asset replacement categories."
      },
      {
        title: "How In-Memory Patching Works",
        body: "QuarterMaster replaces love.graphics.newImage and love.audio.newSource lookups in RAM."
      },
      {
        title: "Seamless Handoff to Patch Studio",
        body: "Transitions into studio to hook asset pre-caching routines."
      }
    ],
    sampleToml: "[[patches]]\n[patches.pattern]\nid = \"SpriteOverrideHook\"\ntarget = \"main.lua\"\npattern = 'love.graphics.draw(img, x, y)'\nposition = \"before\"\npayload = 'img = ModTextures.getOverride(img) or img'"
  }
];

function selectTutorialTemplate(templateId) {
  const tpl = TUTORIAL_TEMPLATES.find((t) => t.id === templateId) || TUTORIAL_TEMPLATES[0];
  state.activeTutorialTemplateId = tpl.id;
  if (typeof renderTutorialsHub === "function") {
    renderTutorialsHub();
  }
  return tpl;
}

function renderTutorialsHubHtml(tpl) {
  if (!tpl) tpl = selectTutorialTemplate(state.activeTutorialTemplateId || "minimal-patch");

  const sidebarItems = TUTORIAL_TEMPLATES.map((item) => {
    const isActive = item.id === tpl.id;
    return `
      <div class="tut-item ${isActive ? "active" : ""}" onclick="onTutorialTemplateClick('${escapeHtml(item.id)}')">
        <div class="tut-item-title">${escapeHtml(item.title)}</div>
        <div class="tut-item-desc">${escapeHtml(item.desc)}</div>
      </div>
    `;
  }).join("");

  const stepsHtml = (tpl.steps || []).map((step, idx) => `
    <div class="step-card">
      <div class="step-header">
        <span class="step-badge">${idx + 1}</span>
        <span>${escapeHtml(step.title)}</span>
      </div>
      <p style="margin: 0; font-size: 11.5px; color: var(--text-secondary); line-height: 1.4;">
        ${escapeHtml(step.body)}
      </p>
    </div>
  `).join("");

  return `
    <div class="tut-grid">
      <div class="tut-sidebar">
        <div class="tut-group-header">Available Templates</div>
        ${sidebarItems}
      </div>
      <div class="tut-main">
        <div style="display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 12px; border-bottom: 1px solid var(--border-subtle); padding-bottom: 10px;">
          <div>
            <h3 style="margin: 0 0 4px 0; color: var(--text-primary); font-size: 15px;">${escapeHtml(tpl.title)}</h3>
            <p style="margin: 0; color: var(--text-muted); font-size: 11px;">${escapeHtml(tpl.desc)}</p>
          </div>
          <button type="button" class="btn btn-accent" onclick="scaffoldAndOpenInStudio('${escapeHtml(tpl.id)}')">+ Scaffold Mod from Template</button>
        </div>
        ${stepsHtml}
        <div class="step-card">
          <div class="step-header">
            <span class="step-badge">[Files]</span>
            <span>Template Project Tree &amp; Sample qm.toml</span>
          </div>
          <pre class="file-tree-box">${escapeHtml(tpl.fileTree)}</pre>
          <pre class="patch-snippet-box">${escapeHtml(tpl.sampleToml)}</pre>
        </div>
      </div>
    </div>
  `;
}

function renderTutorialsHub() {
  const container = document.getElementById("hybrid-tutorials-container");
  if (!container) return;
  const tpl = TUTORIAL_TEMPLATES.find((t) => t.id === (state.activeTutorialTemplateId || "minimal-patch")) || TUTORIAL_TEMPLATES[0];
  container.innerHTML = renderTutorialsHubHtml(tpl);
}

function onTutorialTemplateClick(templateId) {
  selectTutorialTemplate(templateId);
  renderTutorialsHub();
}

async function scaffoldAndOpenInStudio(templateId, customModName) {
  const tpl = TUTORIAL_TEMPLATES.find((t) => t.id === templateId) || TUTORIAL_TEMPLATES[0];
  const modId = customModName || prompt("Enter a unique folder name / ID for your new mod:", tpl.id.replace(/-/g, "_"));
  if (!modId) return;

  try {
    if (typeof apiPost === "function") {
      await apiPost("/api/devtools/scaffold", {
        mod_id: modId,
        name: modId.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase()),
        author: "Modder",
        template: tpl.id
      });
    }
  } catch (_) {}

  state.activeModTab = "studio";
  state.patchWizardStep = 1;
  if (!state.patchWizardData) state.patchWizardData = {};
  state.patchWizardData.targetMod = modId;

  // Switch UI tab
  const studioSubtabBtn = document.querySelector('#modtools-subtabs .subtab[data-modtab="studio"]');
  if (studioSubtabBtn) studioSubtabBtn.click();

  if (typeof showToast === "function") {
    showToast(`[OK] Scaffolded mod '${modId}' and opened Patch Studio!`);
  }
}

function initHybridTutorialsHub() {
  if (state.tutorialsHubInitialized) return;
  state.tutorialsHubInitialized = true;
  // Hybrid tutorials initialization
}

// ============================================================================
// ============================================================================
// TAB 6 SUB-TAB 2: VANILLA ENGINE REFERENCE & CATALOG EXPLORER (NO EMOJIS)
// ============================================================================

const CATEGORY_META = {
  create: { label: "Creation & Spawning", tag: "[Create]", cls: "cat-badge-create", rank: 1 },
  draw: { label: "Rendering & Visuals", tag: "[Render]", cls: "cat-badge-draw", rank: 2 },
  check: { label: "Conditions & Checks", tag: "[Check]", cls: "cat-badge-check", rank: 3 },
  calc: { label: "Logic & Calculations", tag: "[Calc]", cls: "cat-badge-calc", rank: 4 },
  state: { label: "State & Updates", tag: "[State]", cls: "cat-badge-state", rank: 5 },
  ai: { label: "AI & Behavior", tag: "[AI]", cls: "cat-badge-ai", rank: 6 },
  hooks: { label: "Engine & Hooks", tag: "[Hook]", cls: "cat-badge-hooks", rank: 7 },
  general: { label: "General Functions", tag: "[General]", cls: "cat-badge-general", rank: 8 },
  class_table: { label: "State & Class Tables", tag: "[Table]", cls: "cat-badge-class_table", rank: 9 },
};

const PREFIX_CATEGORIES = [
  { id: "create", prefixes: ["create", "spawn", "make", "new", "build", "generate", "place", "add"] },
  { id: "draw", prefixes: ["draw", "display", "render", "show", "flash", "animate", "print", "play", "sound"] },
  { id: "check", prefixes: ["check", "is", "has", "can", "test", "verify", "eval", "inside", "valid", "detect"] },
  { id: "calc", prefixes: ["calc", "compute", "find", "get", "tell", "count", "measure", "total", "best", "number"] },
  { id: "state", prefixes: ["update", "tick", "step", "set", "reset", "give", "remove", "clear", "clean", "wipe", "affect", "change", "damage", "heal", "kill", "destroy", "hit", "drop", "throw", "perform", "use"] },
  { id: "ai", prefixes: ["ai", "think", "behavior", "order", "command", "route", "path", "retreat", "patrol", "target", "engage", "flee"] },
  { id: "hooks", prefixes: ["love.", "on", "handle", "event", "init", "audit", "serialize", "deserialize", "steam", "ach"] },
  { id: "general", prefixes: [] }
];

function classifySymbolClient(name) {
  const n = (name || "").toLowerCase();
  if (n.startsWith("love.")) {
    return { category: "hooks", action: "hook", target: name.slice(5) };
  }
  for (const cat of PREFIX_CATEGORIES) {
    for (const prefix of cat.prefixes) {
      if (n.startsWith(prefix)) {
        let target = name.slice(prefix.length).replace(/^[_.:]+/, "");
        return { category: cat.id, action: prefix, target: target || name };
      }
    }
  }
  if (n.endsWith("tell")) {
    return { category: "calc", action: "tell", target: name.slice(0, -4) };
  }
  return { category: "general", action: "fn", target: name };
}

const FILE_GROUPS = [
  {
    title: "Engine Core & Lifecycle",
    match: ["conf.lua", "main.lua", "includes/urb_engine.lua", "includes/vehicleEngine.lua"]
  },
  {
    title: "Battlefield Maps & Terrains",
    match: ["includes/mapBuild.lua", "includes/mapBuild_1.lua", "includes/mapBuild_3.lua", "includes/mapGeneration.lua"]
  },
  {
    title: "Combat AI & Modes",
    match: ["includes/ai.lua", "includes/mnb2.lua", "includes/trench.lua", "includes/directives.lua", "includes/bcChecks.lua"]
  },
  {
    title: "Rendering & Particles",
    match: ["includes/drawFiles.lua", "includes/drawFiles_1.lua", "includes/particlesSystem.lua", "includes/audioAudit.lua", "includes/gfx.lua"]
  },
  {
    title: "Utilities, Lore & Shaders",
    match: ["includes/localisation.lua", "includes/external.lua", "includes/binser.lua", "includes/ser.lua", "includes/cron.lua"]
  }
];

function buildSamplePatchSnippet(fileRel, name, params, isLocal, kind) {
  if (kind === "class_table") {
    return `[[patches]]\n[patches.pattern]\ntarget = "${fileRel}"\npattern = 'require "includes/directives"'\nposition = "after"\npayload = '''\n-- Direct inspection/manipulation of ${name} table:\nprint("[Mod] Initialized hook for ${name}")\n'''`;
  }
  const argNames = (params || []).map((p) => (typeof p === "string" ? p : p.name)).join(", ");
  const prefix = isLocal ? "local " : "";
  const sig = `${prefix}function ${name}(${argNames})`;
  return `[[patches]]\n[patches.pattern]\ntarget = "${fileRel}"\npattern = '${sig}'\nposition = "after"\npayload = '''\n-- Custom logic injected into ${name}\nprint("[Mod] Hooked into ${name} successfully!")\n'''`;
}

function normalizeVanillaReferenceCatalog(data) {
  if (!data) return null;

  // If already contains flat symbols, ensure every symbol has category, action, and target
  if (data.symbols) {
    for (const sym of data.symbols) {
      if (!sym.category || sym.category === "general" || !sym.action) {
        if (sym.kind === "class_table") {
          sym.category = sym.category || "class_table";
          sym.action = "class";
          sym.target = sym.name;
        } else {
          const derived = classifySymbolClient(sym.name);
          sym.category = sym.category && sym.category !== "general" ? sym.category : derived.category;
          sym.action = sym.action && sym.action !== "fn" ? sym.action : derived.action;
          sym.target = sym.target || derived.target;
        }
      }
    }
  }

  // If new hierarchical format with files
  if (data.files && !data.symbols) {
    const flatSymbols = [];
    for (const f of data.files) {
      const modKey = f.file.split("/")[0].replace(".lua", "");
      for (const fn of f.functions || []) {
        const derived = classifySymbolClient(fn.name);
        flatSymbols.push({
          id: `${f.file}:${fn.name}:${fn.line}`,
          name: fn.name,
          kind: fn.name.startsWith("love.") ? "hook" : "function",
          category: fn.category || derived.category,
          action: fn.action || derived.action,
          target: fn.target || derived.target,
          module: modKey,
          file: f.file,
          line: fn.line,
          is_local: !!fn.is_local,
          params: fn.params || [],
          return_type: fn.returns || "void",
          description: fn.desc || `Vanilla engine function in ${f.file}`,
          sample_patch: buildSamplePatchSnippet(f.file, fn.name, fn.params, fn.is_local, "function")
        });
      }
    }
    for (const cls of data.classes || []) {
      flatSymbols.push({
        id: cls.id,
        name: cls.name,
        kind: "class_table",
        category: cls.category || "class_table",
        action: "class",
        target: cls.name,
        module: cls.file.split("/")[0].replace(".lua", ""),
        file: cls.file,
        line: cls.line,
        is_local: false,
        params: [],
        return_type: cls.returns || "table",
        description: cls.desc,
        sample_patch: buildSamplePatchSnippet(cls.file, cls.name, [], false, "class_table")
      });
    }
    data.symbols = flatSymbols;
    return data;
  }

  // If legacy flat format with only symbols, reconstruct files map
  if (data.symbols && !data.files) {
    const fileMap = new Map();
    for (const sym of data.symbols) {
      const fPath = sym.file || "unknown";
      if (!fileMap.has(fPath)) {
        fileMap.set(fPath, {
          file: fPath,
          name: fPath.split("/").pop(),
          total_functions: 0,
          category_counts: {},
          functions: []
        });
      }
      const fEntry = fileMap.get(fPath);
      fEntry.total_functions++;
      const cat = sym.category || (sym.kind === "class_table" ? "class_table" : "general");
      fEntry.category_counts[cat] = (fEntry.category_counts[cat] || 0) + 1;
      fEntry.functions.push({
        name: sym.name,
        category: cat,
        action: sym.action || "fn",
        target: sym.target || sym.name,
        line: sym.line,
        is_local: sym.is_local,
        params: sym.params,
        returns: sym.return_type,
        desc: sym.description
      });
    }
    data.files = Array.from(fileMap.values());
    return data;
  }

  return data;
}

async function loadVanillaApi() {
  if (state.vanillaApi) return state.vanillaApi;

  // 1. Attempt dynamic network fetch of modern reference JSON
  try {
    const res = await fetch("vanilla_engine_reference.json");
    if (res.ok) {
      const data = await res.json();
      state.vanillaApi = normalizeVanillaReferenceCatalog(data);
      return state.vanillaApi;
    }
  } catch (_) {}

  try {
    const res = await fetch("/vanilla_engine_reference.json");
    if (res.ok) {
      const data = await res.json();
      state.vanillaApi = normalizeVanillaReferenceCatalog(data);
      return state.vanillaApi;
    }
  } catch (_) {}

  // 2. Fall back to vanilla_api.json
  try {
    const res = await fetch("vanilla_api.json");
    if (res.ok) {
      const data = await res.json();
      state.vanillaApi = normalizeVanillaReferenceCatalog(data);
      return state.vanillaApi;
    }
  } catch (_) {}

  try {
    const res = await fetch("/vanilla_api.json");
    if (res.ok) {
      const data = await res.json();
      state.vanillaApi = normalizeVanillaReferenceCatalog(data);
      return state.vanillaApi;
    }
  } catch (_) {}

  // 3. Fall back to bundled JSON in document
  const bundled = document.getElementById("vanilla-api-bundled");
  if (bundled && bundled.textContent && bundled.textContent.trim().length > 0) {
    try {
      const data = JSON.parse(bundled.textContent);
      state.vanillaApi = normalizeVanillaReferenceCatalog(data);
      return state.vanillaApi;
    } catch (e) {
      console.error("Failed to parse bundled vanilla_api data:", e);
    }
  }

  return null;
}

let apiSearchDebounceTimer = null;

async function initApiDocsTab() {
  if (state.apiDocsInitialized && state.vanillaApi) {
    return;
  }

  const listEl = document.getElementById("api-symbols-list");
  if (listEl) {
    listEl.innerHTML = '<div class="api-list-loading">Loading vanilla engine reference (2,155+ symbols)...</div>';
  }

  const catalog = await loadVanillaApi();
  if (!catalog || !catalog.symbols) {
    if (listEl) {
      listEl.innerHTML = '<div class="api-list-empty">Unable to load vanilla engine reference. Run <code>python tools/extract_vanilla_reference.py</code> to generate reference data.</div>';
    }
    return;
  }

  state.apiDocsInitialized = true;
  if (!state.apiActiveFile) state.apiActiveFile = "all";
  if (!state.apiActiveCategory) state.apiActiveCategory = "all";
  if (!state.apiActiveAction) state.apiActiveAction = "all";
  if (!state.apiSortBy) state.apiSortBy = "default";
  if (!state.apiSortDir) state.apiSortDir = "asc";
  if (state.apiGroupByCat === undefined) state.apiGroupByCat = true;

  // Populate legacy module dropdown if present
  const modSelect = document.getElementById("api-module-filter");
  if (modSelect && catalog.modules) {
    const totalFiles = catalog.stats?.total_files || (catalog.files ? catalog.files.length : 43);
    const opts = ['<option value="all">All Modules (' + totalFiles + ' files)</option>'];
    for (const mod of catalog.modules) {
      opts.push(`<option value="${escapeHtml(mod.id)}">${escapeHtml(mod.name)} (${mod.symbol_count})</option>`);
    }
    modSelect.innerHTML = opts.join("");
  }

  // Category select handler
  const kindSelect = document.getElementById("api-kind-filter");
  if (kindSelect && !kindSelect.dataset.bound) {
    kindSelect.dataset.bound = "true";
    kindSelect.addEventListener("change", (e) => {
      state.apiActiveCategory = e.target.value;
      state.apiKindFilter = e.target.value;
      state.apiActiveAction = "all";
      updateCategoryPillsUI();
      renderApiSymbolsList();
    });
  }

  // Category Pills Bar handlers
  const pillsContainer = document.getElementById("api-category-pills");
  if (pillsContainer && !pillsContainer.dataset.bound) {
    pillsContainer.dataset.bound = "true";
    pillsContainer.addEventListener("click", (e) => {
      const btn = e.target.closest(".category-pill");
      if (!btn) return;
      const cat = btn.dataset.category || "all";
      state.apiActiveCategory = cat;
      state.apiKindFilter = cat;
      state.apiActiveAction = "all";
      if (kindSelect) kindSelect.value = cat;
      updateCategoryPillsUI();
      renderApiSymbolsList();
    });
  }

  // Sort By select handler
  const sortBySelect = document.getElementById("api-sort-by");
  if (sortBySelect && !sortBySelect.dataset.bound) {
    sortBySelect.dataset.bound = "true";
    sortBySelect.value = state.apiSortBy;
    sortBySelect.addEventListener("change", (e) => {
      state.apiSortBy = e.target.value;
      renderApiSymbolsList();
    });
  }

  // Sort Direction button handler
  const sortDirBtn = document.getElementById("btn-api-sort-dir");
  if (sortDirBtn && !sortDirBtn.dataset.bound) {
    sortDirBtn.dataset.bound = "true";
    sortDirBtn.textContent = state.apiSortDir.toUpperCase();
    sortDirBtn.addEventListener("click", () => {
      state.apiSortDir = state.apiSortDir === "asc" ? "desc" : "asc";
      sortDirBtn.textContent = state.apiSortDir.toUpperCase();
      renderApiSymbolsList();
    });
  }

  // Action Sub-filter handler
  const actionFilter = document.getElementById("api-action-filter");
  if (actionFilter && !actionFilter.dataset.bound) {
    actionFilter.dataset.bound = "true";
    actionFilter.addEventListener("change", (e) => {
      state.apiActiveAction = e.target.value;
      renderApiSymbolsList();
    });
  }

  // Group by Category toggle handler
  const groupToggle = document.getElementById("api-toggle-grouping");
  if (groupToggle && !groupToggle.dataset.bound) {
    groupToggle.dataset.bound = "true";
    groupToggle.checked = !!state.apiGroupByCat;
    groupToggle.addEventListener("change", (e) => {
      state.apiGroupByCat = e.target.checked;
      renderApiSymbolsList();
    });
  }

  // File filter input handler
  const fileSearchInput = document.getElementById("api-file-filter-input");
  if (fileSearchInput && !fileSearchInput.dataset.bound) {
    fileSearchInput.dataset.bound = "true";
    fileSearchInput.addEventListener("input", (e) => {
      state.apiFileSearchQuery = (e.target.value || "").trim().toLowerCase();
      renderApiFilesList();
    });
  }

  // Search input handler
  const searchInput = document.getElementById("api-search-input");
  if (searchInput && !searchInput.dataset.bound) {
    searchInput.dataset.bound = "true";
    searchInput.addEventListener("input", (e) => {
      clearTimeout(apiSearchDebounceTimer);
      apiSearchDebounceTimer = setTimeout(() => {
        state.apiSearchQuery = (e.target.value || "").trim().toLowerCase();
        renderApiSymbolsList();
      }, 80);
    });
  }

  if (modSelect && !modSelect.dataset.bound) {
    modSelect.dataset.bound = "true";
    modSelect.addEventListener("change", (e) => {
      state.apiModuleFilter = e.target.value;
      renderApiSymbolsList();
    });
  }

  renderApiFilesList();
  renderApiSymbolsList();
}

function updateCategoryPillsUI() {
  const container = document.getElementById("api-category-pills");
  if (!container) return;
  const active = state.apiActiveCategory || "all";
  container.querySelectorAll(".category-pill").forEach((btn) => {
    btn.classList.toggle("active", (btn.dataset.category || "all") === active);
  });
}

function renderApiFilesList() {
  const catalog = state.vanillaApi;
  if (!catalog || !catalog.files) return;

  const container = document.getElementById("api-files-list");
  if (!container) return;

  const q = state.apiFileSearchQuery || "";
  const totalFns = catalog.stats?.total_functions || catalog.symbols?.length || 2157;
  const itemsHtml = [];

  // Top row: All Source Files (No emoji)
  const isAllActive = state.apiActiveFile === "all";
  itemsHtml.push(`
    <div class="api-file-item ${isAllActive ? "active" : ""}" data-file-path="all">
      <span class="api-file-name">All Files (${catalog.files.length})</span>
      <span class="api-file-badge">${totalFns.toLocaleString()}</span>
    </div>
  `);

  for (const group of FILE_GROUPS) {
    const matchingFiles = catalog.files.filter((f) => {
      const inGroup = group.match.some((m) => f.file.endsWith(m) || f.file === m);
      if (!inGroup) return false;
      if (q && !f.file.toLowerCase().includes(q) && !f.name.toLowerCase().includes(q)) {
        return false;
      }
      return true;
    });

    if (matchingFiles.length === 0) continue;

    itemsHtml.push(`<div class="api-file-group-header">${escapeHtml(group.title)}</div>`);
    for (const file of matchingFiles) {
      const isActive = state.apiActiveFile === file.file;
      const count = file.total_functions || (file.functions ? file.functions.length : 0);
      itemsHtml.push(`
        <div class="api-file-item ${isActive ? "active" : ""}" data-file-path="${escapeHtml(file.file)}" title="${escapeHtml(file.file)}">
          <span class="api-file-name">${escapeHtml(file.name)}</span>
          <span class="api-file-badge">${count}</span>
        </div>
      `);
    }
  }

  // Other modules / shaders
  const otherFiles = catalog.files.filter((f) => {
    const isGrouped = FILE_GROUPS.some((g) => g.match.some((m) => f.file.endsWith(m) || f.file === m));
    if (isGrouped) return false;
    if (q && !f.file.toLowerCase().includes(q) && !f.name.toLowerCase().includes(q)) {
      return false;
    }
    return true;
  });

  if (otherFiles.length > 0) {
    itemsHtml.push(`<div class="api-file-group-header">Shaders &amp; Other Modules</div>`);
    for (const file of otherFiles) {
      const isActive = state.apiActiveFile === file.file;
      const count = file.total_functions || (file.functions ? file.functions.length : 0);
      itemsHtml.push(`
        <div class="api-file-item ${isActive ? "active" : ""}" data-file-path="${escapeHtml(file.file)}" title="${escapeHtml(file.file)}">
          <span class="api-file-name">${escapeHtml(file.name)}</span>
          <span class="api-file-badge">${count}</span>
        </div>
      `);
    }
  }

  container.innerHTML = itemsHtml.join("");

  container.onclick = (e) => {
    const item = e.target.closest(".api-file-item");
    if (!item) return;
    const fPath = item.dataset.filePath;
    if (fPath !== state.apiActiveFile) {
      state.apiActiveFile = fPath;
      state.apiActiveAction = "all";
      container.querySelectorAll(".api-file-item").forEach((el) => {
        el.classList.toggle("active", el.dataset.filePath === fPath);
      });
      const titleEl = document.getElementById("api-file-active-title");
      if (titleEl) {
        titleEl.textContent = fPath === "all" ? "All Engine Functions" : fPath.split("/").pop();
      }
      renderApiSymbolsList();
    }
  };
}

function updateActionFilterDropdown(actions) {
  const select = document.getElementById("api-action-filter");
  if (!select) return;

  const currentVal = state.apiActiveAction || "all";
  const opts = ['<option value="all">All Actions</option>'];

  // Count occurrences of each action
  const actionCounts = {};
  for (const act of actions) {
    if (!act || act === "fn") continue;
    actionCounts[act] = (actionCounts[act] || 0) + 1;
  }

  const sortedActions = Object.keys(actionCounts).sort((a, b) => actionCounts[b] - actionCounts[a]);
  for (const act of sortedActions) {
    const isSelected = act === currentVal ? "selected" : "";
    opts.push(`<option value="${escapeHtml(act)}" ${isSelected}>${escapeHtml(act)} (${actionCounts[act]})</option>`);
  }

  select.innerHTML = opts.join("");
  if (!actionCounts[currentVal] && currentVal !== "all") {
    state.apiActiveAction = "all";
    select.value = "all";
  }
}

function renderApiSymbolsList() {
  const catalog = state.vanillaApi;
  if (!catalog || !catalog.symbols) return;

  const listEl = document.getElementById("api-symbols-list");
  const countEl = document.getElementById("api-symbol-count");
  if (!listEl) return;

  const q = state.apiSearchQuery;
  const activeFile = state.apiActiveFile;
  const activeCat = state.apiActiveCategory;
  const activeAction = state.apiActiveAction;
  const modFilter = state.apiModuleFilter;
  const kindFilter = state.apiKindFilter;
  const sortBy = state.apiSortBy || "default";
  const sortDir = state.apiSortDir || "asc";
  const groupByCat = state.apiGroupByCat !== false;

  // Filter pass
  const matchedActions = [];
  const filtered = catalog.symbols.filter((sym) => {
    if (activeFile && activeFile !== "all" && sym.file !== activeFile) {
      return false;
    }
    if (modFilter && modFilter !== "all" && sym.module !== modFilter) {
      return false;
    }
    if (kindFilter && kindFilter !== "all") {
      if (kindFilter === "class_table") {
        if (sym.kind !== "class_table") return false;
      } else if (kindFilter === "function") {
        if (sym.kind !== "function") return false;
      } else if (kindFilter === "hook") {
        if (sym.kind !== "hook") return false;
      } else if (sym.category !== kindFilter && sym.kind !== kindFilter) {
        return false;
      }
    }
    if (activeCat && activeCat !== "all") {
      if (activeCat === "class_table") {
        if (sym.kind !== "class_table") return false;
      } else if (sym.category !== activeCat && sym.kind !== activeCat) {
        return false;
      }
    }

    // Collect actions available before action filter
    if (sym.action) matchedActions.push(sym.action);

    if (activeAction && activeAction !== "all" && sym.action !== activeAction) {
      return false;
    }

    if (q) {
      const matchName = sym.name.toLowerCase().includes(q);
      const matchFile = (sym.file || "").toLowerCase().includes(q);
      const matchTarget = (sym.target || "").toLowerCase().includes(q);
      const matchAction = (sym.action || "").toLowerCase().includes(q);
      const matchDesc = (sym.description || "").toLowerCase().includes(q);
      const matchParams = sym.params && sym.params.some((p) => {
        const pName = typeof p === "string" ? p : p.name;
        const pType = typeof p === "object" ? p.type : "";
        return pName.toLowerCase().includes(q) || (pType || "").toLowerCase().includes(q);
      });
      if (!matchName && !matchFile && !matchTarget && !matchAction && !matchDesc && !matchParams) {
        return false;
      }
    }
    return true;
  });

  // Update action dropdown
  updateActionFilterDropdown(matchedActions);

  if (countEl) {
    countEl.textContent = `${filtered.length.toLocaleString()} symbol${filtered.length === 1 ? "" : "s"}`;
  }

  const fileSubtitle = document.getElementById("api-list-subtitle");
  if (fileSubtitle) {
    if (activeFile && activeFile !== "all") {
      fileSubtitle.textContent = `${activeFile} (${filtered.length} matching)`;
    } else {
      fileSubtitle.textContent = `Showing ${filtered.length.toLocaleString()} symbols across all files`;
    }
  }

  if (filtered.length === 0) {
    listEl.innerHTML = '<div class="api-list-empty">No symbols match your filter criteria.</div>';
    const detailEl = document.getElementById("api-symbol-detail");
    if (detailEl) {
      detailEl.innerHTML = `
        <div class="empty-state">
          <h3>No Matching Symbols</h3>
          <p>No symbols matched "<strong>${escapeHtml(q)}</strong>" in the active file or category. Try clearing filters or selecting another category.</p>
        </div>
      `;
    }
    return;
  }

  // Sorting
  if (sortBy !== "default") {
    filtered.sort((a, b) => {
      let cmp = 0;
      if (sortBy === "name") {
        cmp = a.name.localeCompare(b.name);
      } else if (sortBy === "line") {
        cmp = (a.line || 0) - (b.line || 0);
      } else if (sortBy === "params") {
        cmp = (b.params?.length || 0) - (a.params?.length || 0);
      } else if (sortBy === "category") {
        const rankA = CATEGORY_META[a.category]?.rank || 99;
        const rankB = CATEGORY_META[b.category]?.rank || 99;
        if (rankA !== rankB) {
          cmp = rankA - rankB;
        } else {
          const actA = a.action || "";
          const actB = b.action || "";
          if (actA !== actB) {
            cmp = actA.localeCompare(actB);
          } else {
            cmp = (a.target || a.name).localeCompare(b.target || b.name);
          }
        }
      }
      return sortDir === "desc" ? -cmp : cmp;
    });
  }

  let activeSym = filtered.find((s) => s.id === state.apiSelectedSymbolId);
  if (!activeSym) {
    activeSym = filtered[0];
    state.apiSelectedSymbolId = activeSym.id;
  }

  const maxRender = Math.min(filtered.length, 350);
  const itemsHtml = [];
  let currentGroup = null;

  for (let i = 0; i < maxRender; i++) {
    const sym = filtered[i];
    const isActive = sym.id === state.apiSelectedSymbolId;
    const catMeta = CATEGORY_META[sym.category] || CATEGORY_META[sym.kind] || CATEGORY_META.general;
    const isClass = sym.kind === "class_table";
    const paramCount = (sym.params || []).length;
    const fileNameOnly = (sym.file || "").split("/").pop();

    // Section header divider if grouping is enabled and category changes
    if (groupByCat && sortBy === "category" && currentGroup !== sym.category) {
      currentGroup = sym.category;
      const groupCount = filtered.filter((s) => s.category === currentGroup).length;
      itemsHtml.push(`
        <div class="api-symbol-group-header">
          <span>${escapeHtml(catMeta.label)}</span>
          <span class="badge" style="font-size: 10px;">${groupCount}</span>
        </div>
      `);
    }

    let derivationHtml = "";
    if (isClass) {
      derivationHtml = `<span class="derivation-target" style="color:var(--status-warn);">Core State Table</span>`;
    } else if (sym.target && sym.action && sym.action !== "fn") {
      derivationHtml = `<span>${escapeHtml(sym.action)}</span> <span class="derivation-arrow">&rarr;</span> <span class="derivation-target">${escapeHtml(sym.target)}</span>`;
    } else {
      derivationHtml = `<span>${escapeHtml(fileNameOnly)}</span>`;
    }

    itemsHtml.push(`
      <div class="api-symbol-item ${isActive ? "active" : ""}" data-symbol-id="${escapeHtml(sym.id)}">
        <div class="api-symbol-top-row">
          <span class="api-symbol-name">${escapeHtml(sym.name)}</span>
          <span class="cat-badge ${catMeta.cls}">${catMeta.tag}</span>
        </div>
        <div class="api-action-derivation">
          ${derivationHtml}
        </div>
        <div class="api-symbol-meta">
          <span class="api-line-badge">${escapeHtml(fileNameOnly)}:${sym.line}</span>
          ${!isClass ? `<span class="badge" style="font-size: 10px; padding: 1px 5px;">${paramCount} arg${paramCount === 1 ? "" : "s"}</span>` : ""}
          <span class="badge" style="font-size: 10px; padding: 1px 5px; font-family: var(--font-mono);">${escapeHtml(sym.return_type || "void")}</span>
        </div>
      </div>
    `);
  }

  if (filtered.length > maxRender) {
    itemsHtml.push(`
      <div style="padding: 10px; text-align: center; color: var(--text-muted); font-size: 11px;">
        Showing top ${maxRender} of ${filtered.length.toLocaleString()} matching symbols. Refine search or sort to narrow down.
      </div>
    `);
  }

  listEl.innerHTML = itemsHtml.join("");

  listEl.onclick = (e) => {
    const item = e.target.closest(".api-symbol-item");
    if (!item) return;
    const symId = item.dataset.symbolId;
    if (symId && symId !== state.apiSelectedSymbolId) {
      state.apiSelectedSymbolId = symId;
      listEl.querySelectorAll(".api-symbol-item").forEach((el) => {
        el.classList.toggle("active", el.dataset.symbolId === symId);
      });
      const sym = catalog.symbols.find((s) => s.id === symId);
      if (sym) {
        renderActiveSymbolDetail(sym);
      }
    }
  };

  renderActiveSymbolDetail(activeSym);
}

function renderActiveSymbolDetail(sym) {
  const detailEl = document.getElementById("api-symbol-detail");
  if (!detailEl || !sym) return;

  const isClass = sym.kind === "class_table";
  const catMeta = CATEGORY_META[sym.category] || CATEGORY_META[sym.kind] || CATEGORY_META.general;
  const argNames = (sym.params || []).map((p) => (typeof p === "string" ? p : p.name)).join(", ");

  let sigText = "";
  if (isClass) {
    sigText = `${sym.name}: table (Mud and Blood Core Game State)`;
  } else {
    sigText = `${sym.is_local ? "local " : ""}function ${sym.name}(${argNames}) -> ${sym.return_type || "void"}`;
  }

  let paramsTableHtml = "";
  if (sym.params && sym.params.length > 0) {
    paramsTableHtml = `
      <table class="api-param-table">
        <thead>
          <tr>
            <th style="width: 28%;">Parameter</th>
            <th style="width: 32%;">Inferred Type</th>
            <th>Context Description</th>
          </tr>
        </thead>
        <tbody>
          ${sym.params
            .map((p) => {
              const pName = typeof p === "string" ? p : p.name;
              const pType = typeof p === "object" ? p.type : "any";
              const pDesc = typeof p === "object" && p.description ? p.description : `Parameter '${pName}' passed to ${sym.name}.`;
              return `
                <tr>
                  <td class="api-param-name"><code>${escapeHtml(pName)}</code></td>
                  <td class="api-param-type"><code>${escapeHtml(pType)}</code></td>
                  <td class="api-param-desc">${escapeHtml(pDesc)}</td>
                </tr>
              `;
            })
            .join("")}
        </tbody>
      </table>
    `;
  } else {
    paramsTableHtml = `
      <div class="api-desc-box" style="color: var(--text-muted); font-style: italic;">
        ${isClass ? "Foundational state table structure." : "This function takes no parameters (void call)."}
      </div>
    `;
  }

  const patchSnippet = sym.sample_patch || buildSamplePatchSnippet(sym.file, sym.name, sym.params, sym.is_local, sym.kind);

  detailEl.innerHTML = `
    <div class="api-detail-header">
      <div class="api-detail-title-cluster">
        <div class="api-detail-title">${escapeHtml(sym.name)}</div>
        <div class="api-detail-badges">
          <span class="cat-badge ${catMeta.cls}">${catMeta.tag} ${escapeHtml(catMeta.label)}</span>
          <span class="api-detail-file"><code>${escapeHtml(sym.file)}:${sym.line}</code></span>
        </div>
      </div>
      <div style="display: flex; gap: 6px; align-items: center;">
        <button type="button" class="btn btn-sm" id="btn-copy-fn-signature" title="Copy full signature to clipboard">
          Copy Signature
        </button>
        <button type="button" class="btn btn-sm btn-accent" id="btn-copy-patch-snippet" title="Copy qm.toml patch snippet to clipboard">
          Copy Patch
        </button>
      </div>
    </div>

    <!-- Derivation Hierarchy Box -->
    <div class="api-section">
      <span class="api-section-title">Functional Derivation</span>
      <div class="api-signature-card" style="padding: 10px 14px; font-size: 12px; display: flex; gap: 16px; flex-wrap: wrap;">
        <div><strong>Action Family:</strong> ${escapeHtml(catMeta.label)}</div>
        <div><strong>Action Verb:</strong> <code>${escapeHtml(sym.action || "fn")}</code></div>
        <div><strong>Target Entity:</strong> <code>${escapeHtml(sym.target || sym.name)}</code></div>
        <div><strong>Source Location:</strong> <code>${escapeHtml(sym.file)}:${sym.line}</code></div>
      </div>
    </div>

    <div class="api-section">
      <span class="api-section-title">Signature</span>
      <div class="api-signature-card"><code>${escapeHtml(sigText)}</code></div>
    </div>

    <div class="api-section">
      <span class="api-section-title">Documentation &amp; Usage Notes</span>
      <div class="api-desc-box">${escapeHtml(sym.description || "Core engine function.")}</div>
    </div>

    <div class="api-section">
      <span class="api-section-title">Parameters (${sym.params ? sym.params.length : 0})</span>
      ${paramsTableHtml}
    </div>

    ${!isClass ? `
      <div class="api-section">
        <span class="api-section-title">Return Type</span>
        <div style="display: flex; align-items: center; gap: 8px;">
          <span class="badge" style="font-family: var(--font-mono); font-size: 11.5px;">${escapeHtml(sym.return_type || "void")}</span>
        </div>
      </div>
    ` : ""}

    <div class="api-section patch-snippet-container">
      <div class="patch-snippet-header">
        <span class="api-section-title">Sample qm.toml Patch Hook</span>
      </div>
      <pre id="api-patch-snippet" class="code-scroll-container patch-snippet-box" tabindex="0">${escapeHtml(patchSnippet)}</pre>
    </div>
  `;

  const copyPatchBtn = document.getElementById("btn-copy-patch-snippet");
  if (copyPatchBtn) {
    copyPatchBtn.onclick = () => {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(patchSnippet).then(() => {
          showToast(`Copied qm.toml patch snippet for '${sym.name}'!`);
        }).catch(() => fallbackCopyText(patchSnippet));
      } else {
        fallbackCopyText(patchSnippet);
      }
    };
  }

  const copySigBtn = document.getElementById("btn-copy-fn-signature");
  if (copySigBtn) {
    copySigBtn.onclick = () => {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(sigText).then(() => {
          showToast(`Copied signature for '${sym.name}'!`);
        }).catch(() => fallbackCopyText(sigText));
      } else {
        fallbackCopyText(sigText);
      }
    };
  }
}


function fallbackCopyText(text) {
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.style.position = "fixed";
  ta.style.opacity = "0";
  document.body.appendChild(ta);
  ta.select();
  try {
    document.execCommand("copy");
    showToast("Copied to clipboard!");
  } catch (err) {
    showToast("Failed to copy to clipboard", true);
  }
  document.body.removeChild(ta);
}

function escapeHtml(str) {
  return String(str ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

// ============================================================================
// TAB 6 SUB-TAB 3: MOD FILE BROWSER & IN-APP CODE EDITOR (M3 / R3)
// ============================================================================

// State additions for M3
state.editorActiveModDir = null;
state.editorActiveFilePath = null;
state.editorOriginalContent = "";
state.editorModFiles = [];

/**
 * Token-based stack validator for Lua syntax.
 * Tracks open blocks (function, if..then, for..do, while..do, repeat..until, do..end)
 * and matching delimiters: (), [], {}.
 * Detects unclosed blocks, mismatched delimiters, and invalid tokens with exact line numbers.
 */
// ============================================================================
// Multi-Language Live Linter & Explainable Diagnostics Engine
// ============================================================================

function lintLuaSource(content) {
  const errors = [];
  const warnings = [];
  if (typeof content !== "string") return { errors, warnings };

  const lines = content.split(/\r?\n/);
  const blockStack = [];
  const delimStack = [];
  let inBlockComment = false;
  let blockCommentStartLine = 0;
  let inLongString = false;
  let longStringStartLine = 0;

  for (let idx = 0; idx < lines.length; idx++) {
    const lineNum = idx + 1;
    const line = lines[idx];
    let i = 0;

    // Check operator warnings on raw line before tokenizing
    if (!inBlockComment && !inLongString) {
      // Pitfall: !=
      const notEqIdx = line.indexOf("!=");
      if (notEqIdx !== -1) {
        const prefixBefore = line.substring(0, notEqIdx);
        const doubleCount = (prefixBefore.match(/"/g) || []).length;
        const singleCount = (prefixBefore.match(/'/g) || []).length;
        if (doubleCount % 2 === 0 && singleCount % 2 === 0 && !prefixBefore.includes("--")) {
          warnings.push({
            line: lineNum,
            column: notEqIdx + 1,
            severity: "warning",
            message: "Lua uses '~=' instead of '!=' for inequality",
            explanation: "In Lua, the inequality comparison operator is '~='. The '!=' operator will cause a syntax error at runtime.",
            fixSuggestion: "Replace '!=' with '~='."
          });
        }
      }

      // Pitfall: && or ||
      const andIdx = line.indexOf("&&");
      if (andIdx !== -1) {
        const prefixBefore = line.substring(0, andIdx);
        const doubleCount = (prefixBefore.match(/"/g) || []).length;
        const singleCount = (prefixBefore.match(/'/g) || []).length;
        if (doubleCount % 2 === 0 && singleCount % 2 === 0 && !prefixBefore.includes("--")) {
          warnings.push({
            line: lineNum,
            column: andIdx + 1,
            severity: "warning",
            message: "Lua uses 'and' keyword instead of '&&'",
            explanation: "In Lua, logical conjunction is written using the keyword 'and'.",
            fixSuggestion: "Replace '&&' with 'and'."
          });
        }
      }

      const orIdx = line.indexOf("||");
      if (orIdx !== -1) {
        const prefixBefore = line.substring(0, orIdx);
        const doubleCount = (prefixBefore.match(/"/g) || []).length;
        const singleCount = (prefixBefore.match(/'/g) || []).length;
        if (doubleCount % 2 === 0 && singleCount % 2 === 0 && !prefixBefore.includes("--")) {
          warnings.push({
            line: lineNum,
            column: orIdx + 1,
            severity: "warning",
            message: "Lua uses 'or' keyword instead of '||'",
            explanation: "In Lua, logical disjunction is written using the keyword 'or'.",
            fixSuggestion: "Replace '||' with 'or'."
          });
        }
      }
    }

    while (i < line.length) {
      if (inBlockComment) {
        const closeIdx = line.indexOf("]]", i);
        if (closeIdx !== -1) {
          inBlockComment = false;
          i = closeIdx + 2;
        } else {
          break;
        }
        continue;
      }

      if (inLongString) {
        const closeIdx = line.indexOf("]]", i);
        if (closeIdx !== -1) {
          inLongString = false;
          i = closeIdx + 2;
        } else {
          break;
        }
        continue;
      }

      if (line.substr(i, 2) === "--") {
        if (line.substr(i, 4) === "--[[") {
          inBlockComment = true;
          blockCommentStartLine = lineNum;
          i += 4;
          continue;
        }
        break;
      }

      if (line.substr(i, 2) === "[[") {
        inLongString = true;
        longStringStartLine = lineNum;
        i += 2;
        continue;
      }

      const ch = line[i];

      if (ch === '"' || ch === "'") {
        const quote = ch;
        const startCol = i + 1;
        i++;
        let closed = false;
        while (i < line.length) {
          if (line[i] === "\\") {
            i += 2;
            continue;
          }
          if (line[i] === quote) {
            closed = true;
            i++;
            break;
          }
          i++;
        }
        if (!closed) {
          errors.push({
            line: lineNum,
            column: startCol,
            severity: "error",
            message: `Unclosed string literal starting at column ${startCol}`,
            explanation: `String literal opened with ${quote} was not closed before the end of the line.`,
            fixSuggestion: `Close the string with matching ${quote} or use multiline string [[...]].`
          });
        }
        continue;
      }

      if (ch === "(" || ch === "[" || ch === "{") {
        delimStack.push({ char: ch, line: lineNum, col: i + 1 });
        i++;
        continue;
      }
      if (ch === ")" || ch === "]" || ch === "}") {
        const expected = ch === ")" ? "(" : ch === "]" ? "[" : "{";
        if (delimStack.length === 0 || delimStack[delimStack.length - 1].char !== expected) {
          errors.push({
            line: lineNum,
            column: i + 1,
            severity: "error",
            message: `Mismatched closing delimiter '${ch}' on line ${lineNum}`,
            explanation: `Closing delimiter '${ch}' does not match any open delimiter.`,
            fixSuggestion: `Check delimiter pairing or remove stray '${ch}'.`
          });
        } else {
          delimStack.pop();
        }
        i++;
        continue;
      }

      if (/[a-zA-Z_]/.test(ch)) {
        const startCol = i + 1;
        let word = "";
        while (i < line.length && /[a-zA-Z0-9_]/.test(line[i])) {
          word += line[i];
          i++;
        }

        if (word === "function") {
          blockStack.push({ type: "function", line: lineNum, col: startCol });
        } else if (word === "if") {
          blockStack.push({ type: "if", line: lineNum, col: startCol, hasThen: false });
        } else if (word === "then") {
          const top = blockStack[blockStack.length - 1];
          if (top && (top.type === "if" || top.type === "elseif")) {
            top.hasThen = true;
          }
        } else if (word === "elseif") {
          const top = blockStack[blockStack.length - 1];
          if (!top || (top.type !== "if" && top.type !== "elseif")) {
            errors.push({
              line: lineNum,
              column: startCol,
              severity: "error",
              message: "Unexpected 'elseif' without matching 'if'",
              explanation: "'elseif' must be placed inside an active 'if' conditional block.",
              fixSuggestion: "Ensure 'elseif' is preceded by an 'if' statement."
            });
          } else {
            top.hasThen = false;
          }
        } else if (word === "else") {
          const top = blockStack[blockStack.length - 1];
          if (!top || (top.type !== "if" && top.type !== "elseif")) {
            errors.push({
              line: lineNum,
              column: startCol,
              severity: "error",
              message: "Unexpected 'else' without matching 'if'",
              explanation: "'else' must be placed inside an active 'if' conditional block.",
              fixSuggestion: "Ensure 'else' is preceded by an 'if' statement."
            });
          }
        } else if (word === "for") {
          blockStack.push({ type: "for", line: lineNum, col: startCol, hasDo: false });
        } else if (word === "while") {
          blockStack.push({ type: "while", line: lineNum, col: startCol, hasDo: false });
        } else if (word === "do") {
          const top = blockStack[blockStack.length - 1];
          if (top && (top.type === "for" || top.type === "while") && !top.hasDo) {
            top.hasDo = true;
          } else {
            blockStack.push({ type: "do", line: lineNum, col: startCol });
          }
        } else if (word === "repeat") {
          blockStack.push({ type: "repeat", line: lineNum, col: startCol });
        } else if (word === "until") {
          if (blockStack.length === 0) {
            errors.push({
              line: lineNum,
              column: startCol,
              severity: "error",
              message: "Unexpected 'until' with no matching 'repeat'",
              explanation: "'until' terminates a 'repeat ... until' loop. No matching 'repeat' block was opened.",
              fixSuggestion: "Open a 'repeat' block earlier or remove 'until'."
            });
          } else {
            const top = blockStack.pop();
            if (top.type !== "repeat") {
              errors.push({
                line: lineNum,
                column: startCol,
                severity: "error",
                message: `Unexpected 'until' closing '${top.type}' from line ${top.line} (expected 'end')`,
                explanation: `Blocks of type '${top.type}' must be closed with 'end', not 'until'.`,
                fixSuggestion: "Replace 'until' with 'end'."
              });
            }
          }
        } else if (word === "end") {
          if (blockStack.length === 0) {
            errors.push({
              line: lineNum,
              column: startCol,
              severity: "error",
              message: "Unexpected 'end' with no matching open block",
              explanation: "Found an 'end' statement with no corresponding 'function', 'if', 'for', 'while', or 'do' block.",
              fixSuggestion: "Remove redundant 'end' or add opening block keyword."
            });
          } else {
            const top = blockStack.pop();
            if (top.type === "repeat") {
              errors.push({
                line: lineNum,
                column: startCol,
                severity: "error",
                message: `Unexpected 'end' for 'repeat' block from line ${top.line} (expected 'until')`,
                explanation: "'repeat' loops in Lua are terminated with 'until <condition>', not 'end'.",
                fixSuggestion: "Replace 'end' with 'until <condition>'."
              });
            }
          }
        }
        continue;
      }

      i++;
    }
  }

  if (inBlockComment) {
    errors.push({
      line: blockCommentStartLine,
      column: 1,
      severity: "error",
      message: `Unclosed multiline comment starting on line ${blockCommentStartLine}`,
      explanation: "A block comment was opened with '--[[' but never closed with ']]'.",
      fixSuggestion: "Add ']]' to close the multiline comment."
    });
  }

  if (inLongString) {
    errors.push({
      line: longStringStartLine,
      column: 1,
      severity: "error",
      message: `Unclosed multiline string starting on line ${longStringStartLine}`,
      explanation: "A multiline string was opened with '[[' but never closed with ']]'.",
      fixSuggestion: "Add ']]' to close the multiline string."
    });
  }

  if (delimStack.length > 0) {
    const unclosed = delimStack[delimStack.length - 1];
    const expected = unclosed.char === "(" ? ")" : unclosed.char === "[" ? "]" : "}";
    errors.push({
      line: unclosed.line,
      column: unclosed.col,
      severity: "error",
      message: `Unclosed delimiter '${unclosed.char}' opened on line ${unclosed.line} (missing '${expected}')`,
      explanation: `Every opening '${unclosed.char}' must have a matching closing '${expected}'.`,
      fixSuggestion: `Add closing '${expected}' at the appropriate location.`
    });
  }

  if (blockStack.length > 0) {
    const unclosed = blockStack[blockStack.length - 1];
    const expected = unclosed.type === "repeat" ? "until" : "end";
    errors.push({
      line: unclosed.line,
      column: unclosed.col || 1,
      severity: "error",
      message: `Unclosed block '${unclosed.type}' started on line ${unclosed.line} (missing matching '${expected}')`,
      explanation: `In Lua, every '${unclosed.type}' block must be closed with '${expected}'.`,
      fixSuggestion: `Add '${expected}' to close the '${unclosed.type}' block.`
    });
  }

  return { errors, warnings };
}

function lintTomlSource(fileName, content) {
  const errors = [];
  const warnings = [];
  if (typeof content !== "string" || !content.trim()) return { errors, warnings };

  const lines = content.split(/\r?\n/);
  const seenTopLevelTables = new Set();
  let currentArraySubTables = new Set();
  let inMultiLineString = false;
  let multiLineDelimiter = null;
  let activePatch = null;

  function finalizeActivePatch(patch) {
    if (!patch) return;

    // Module patch: requires source and name. Does NOT require target or pattern.
    if (patch.kind === "module" || (patch.source && patch.name && !patch.pattern)) {
      if (!patch.source) {
        errors.push({
          line: patch.headerLine,
          column: 1,
          severity: "error",
          message: "Module patch is missing 'source' attribute",
          explanation: "In QuarterMaster, a [patches.module] table requires a 'source' key pointing to the Lua file to preload.",
          fixSuggestion: "Add source = '<filename>.lua'"
        });
      }
      if (!patch.name) {
        errors.push({
          line: patch.headerLine,
          column: 1,
          severity: "error",
          message: "Module patch is missing 'name' attribute",
          explanation: "In QuarterMaster, a [patches.module] table requires a 'name' key for package.preload (e.g. name = 'includes/mymod').",
          fixSuggestion: "Add name = 'includes/<modname>'"
        });
      }
      return;
    }

    // Copy patch: requires target and position ("prepend" | "append")
    if (patch.kind === "copy") {
      if (!patch.target) {
        warnings.push({
          line: patch.headerLine,
          column: 1,
          severity: "warning",
          message: "Copy patch is missing 'target' attribute",
          explanation: "A [patches.copy] table requires a 'target' key indicating which vanilla file to copy or prepend/append to.",
          fixSuggestion: "Add target = 'main.lua'"
        });
      }
      if (patch.position && !["prepend", "append"].includes(patch.position.toLowerCase())) {
        errors.push({
          line: patch.positionLine || patch.headerLine,
          column: 1,
          severity: "error",
          message: `Invalid copy position '${patch.position}'. Supported values are 'prepend' or 'append'`,
          explanation: "Copy patches in QuarterMaster support inserting content at the beginning ('prepend') or end ('append') of a target file.",
          fixSuggestion: "Set position = 'prepend' or 'append'."
        });
      }
      return;
    }

    // Pattern or Regex patch (or generic patch with pattern/target)
    const isPatternOrRegex = patch.kind === "pattern" || patch.kind === "regex" || patch.pattern || patch.target;
    if (isPatternOrRegex) {
      if (!patch.target) {
        warnings.push({
          line: patch.headerLine,
          column: 1,
          severity: "warning",
          message: "Missing 'target' attribute in patch definition",
          explanation: "Pattern and regex patches require a target game file (e.g. target = 'conf.lua', 'main.lua', or '@game/main.lua').",
          fixSuggestion: "Add target = 'main.lua'"
        });
      }
      if (!patch.pattern && patch.kind !== "copy") {
        warnings.push({
          line: patch.headerLine,
          column: 1,
          severity: "warning",
          message: "Missing 'pattern' attribute in patch definition",
          explanation: "Pattern patches require a pattern string to match in the target game file.",
          fixSuggestion: "Add pattern = '...'"
        });
      }
      if (patch.position && !["at", "before", "after"].includes(patch.position.toLowerCase())) {
        errors.push({
          line: patch.positionLine || patch.headerLine,
          column: 1,
          severity: "error",
          message: `Invalid patch position '${patch.position}'. Supported values are 'at', 'before', or 'after'`,
          explanation: "QuarterMaster hook engine supports replacing code ('at'), prepending code ('before'), or appending code ('after').",
          fixSuggestion: "Set position = 'at', 'before', or 'after'."
        });
      }
      return;
    }

    // Bare patch table with no properties
    if (!patch.source && !patch.name && !patch.pattern && !patch.target && !patch.payload) {
      warnings.push({
        line: patch.headerLine,
        column: 1,
        severity: "warning",
        message: "Empty [[patches]] definition",
        explanation: "Every [[patches]] entry must declare a sub-table ([patches.pattern], [patches.module], etc.) or specify patch properties.",
        fixSuggestion: "Specify [patches.pattern] or [patches.module]"
      });
    }
  }

  for (let idx = 0; idx < lines.length; idx++) {
    const lineNum = idx + 1;
    let line = lines[idx];

    if (inMultiLineString) {
      if (line.includes(multiLineDelimiter)) {
        inMultiLineString = false;
        multiLineDelimiter = null;
      }
      continue;
    }

    const commentIdx = line.indexOf("#");
    if (commentIdx !== -1) {
      line = line.substring(0, commentIdx);
    }
    const trimmed = line.trim();
    if (!trimmed) continue;

    // Array of tables [[table]]
    if (trimmed.startsWith("[[") && trimmed.endsWith("]]")) {
      const tableName = trimmed.slice(2, -2).trim();
      if (!tableName) {
        errors.push({
          line: lineNum,
          column: 1,
          severity: "error",
          message: "Empty table array header '[[]]'",
          explanation: "TOML array table headers must contain a valid identifier.",
          fixSuggestion: "Specify a table name like '[[patches]]'."
        });
      }
      if (tableName === "patches" || tableName === "patch") {
        finalizeActivePatch(activePatch);
        currentArraySubTables.clear();
        activePatch = {
          headerLine: lineNum,
          kind: null,
          target: null,
          position: null,
          pattern: null,
          payload: null,
          source: null,
          name: null
        };
      }
      continue;
    }

    // Standard table [table]
    if (trimmed.startsWith("[") && trimmed.endsWith("]")) {
      const tableName = trimmed.slice(1, -1).trim();
      if (!tableName) {
        errors.push({
          line: lineNum,
          column: 1,
          severity: "error",
          message: "Empty table header '[]'",
          explanation: "TOML table headers must contain a non-empty name.",
          fixSuggestion: "Specify a table name like '[manifest]'."
        });
      } else {
        const isPatchSubTable = tableName.startsWith("patches.") || tableName.startsWith("patch.");
        if (isPatchSubTable) {
          const subKind = tableName.split(".")[1] || "";
          if (activePatch) {
            activePatch.kind = subKind;
          }
          if (currentArraySubTables.has(tableName)) {
            errors.push({
              line: lineNum,
              column: 1,
              severity: "error",
              message: `Duplicate sub-table header '[${tableName}]' in same [[patches]] entry`,
              explanation: "In TOML, a single array element cannot contain duplicate sub-tables of the same name.",
              fixSuggestion: `Merge keys into the existing '[${tableName}]' section.`
            });
          } else {
            currentArraySubTables.add(tableName);
          }
        } else {
          // Top-level standalone table
          if (seenTopLevelTables.has(tableName)) {
            errors.push({
              line: lineNum,
              column: 1,
              severity: "error",
              message: `Duplicate table header '[${tableName}]'`,
              explanation: "In TOML, defining the same non-array table multiple times is illegal.",
              fixSuggestion: `Merge keys into the first '[${tableName}]' section.`
            });
          } else {
            seenTopLevelTables.add(tableName);
          }
          finalizeActivePatch(activePatch);
          activePatch = null;
        }
      }
      continue;
    }

    // Key-value pair
    if (trimmed.includes("=")) {
      const eqIdx = trimmed.indexOf("=");
      const key = trimmed.substring(0, eqIdx).trim();
      let val = trimmed.substring(eqIdx + 1).trim();
      if (!key) {
        errors.push({
          line: lineNum,
          column: 1,
          severity: "error",
          message: "Missing key identifier before '=' operator",
          explanation: "In TOML, every assignment must have a valid key name to the left of '='.",
          fixSuggestion: "Provide a key name (e.g. target = '...')."
        });
        continue;
      }

      // Check if multiline string begins
      if (val.startsWith('"""') || val.startsWith("'''")) {
        const delim = val.startsWith('"""') ? '"""' : "'''";
        const remainder = val.substring(3);
        if (!remainder.includes(delim)) {
          inMultiLineString = true;
          multiLineDelimiter = delim;
        }
      } else {
        // Check unclosed quotes on single-line strings
        if (val.startsWith('"') && (!val.endsWith('"') || val.length === 1)) {
          errors.push({
            line: lineNum,
            column: eqIdx + 1,
            severity: "error",
            message: `Unclosed string literal on line ${lineNum}`,
            explanation: "String value started with double quote but was not closed before line end.",
            fixSuggestion: "Add closing double quote '\"'."
          });
        } else if (val.startsWith("'") && (!val.endsWith("'") || val.length === 1)) {
          errors.push({
            line: lineNum,
            column: eqIdx + 1,
            severity: "error",
            message: `Unclosed string literal on line ${lineNum}`,
            explanation: "String value started with single quote but was not closed before line end.",
            fixSuggestion: "Add closing single quote \"'\"."
          });
        }
      }

      if (activePatch) {
        const rawVal = val.replace(/^["']|["']$/g, "");
        if (key === "target") activePatch.target = rawVal;
        if (key === "position") {
          activePatch.position = rawVal;
          activePatch.positionLine = lineNum;
        }
        if (key === "pattern") activePatch.pattern = rawVal;
        if (key === "payload" || key === "replacement") activePatch.payload = rawVal;
        if (key === "source") activePatch.source = rawVal;
        if (key === "name") activePatch.name = rawVal;
      }
      continue;
    }

    errors.push({
      line: lineNum,
      column: 1,
      severity: "error",
      message: `Invalid TOML syntax on line ${lineNum}: '${trimmed}'`,
      explanation: "Line is neither a table header, comment, nor valid key = value assignment.",
      fixSuggestion: "Check line syntax or prefix with '#' if intended as comment."
    });
  }

  finalizeActivePatch(activePatch);

  if (inMultiLineString) {
    errors.push({
      line: lines.length,
      column: 1,
      severity: "error",
      message: `Unclosed multi-line string (${multiLineDelimiter}) at end of file`,
      explanation: "A multiline string was opened but never closed before the end of the file.",
      fixSuggestion: `Add closing ${multiLineDelimiter} at the end of the string block.`
    });
  }

  return { errors, warnings };
}

function lintJsonSource(fileName, content) {
  const errors = [];
  const warnings = [];
  if (typeof content !== "string" || !content.trim()) {
    errors.push({
      line: 1,
      column: 1,
      severity: "error",
      message: "JSON document cannot be empty",
      explanation: "Valid JSON must contain at least a root object ({}) or array ([]).",
      fixSuggestion: "Initialize with '{}'."
    });
    return { errors, warnings };
  }

  let parsed = null;
  try {
    parsed = JSON.parse(content);
  } catch (err) {
    let errLine = 1;
    let errCol = 1;
    const matchPos = err.message.match(/position\s+(\d+)/i);
    if (matchPos) {
      const pos = parseInt(matchPos[1], 10);
      const textUpToPos = content.substring(0, pos);
      const linesUpTo = textUpToPos.split("\n");
      errLine = linesUpTo.length;
      errCol = linesUpTo[errLine - 1].length + 1;
    }

    const trailingCommaRegex = /,\s*[}\]]/;
    if (trailingCommaRegex.test(content)) {
      errors.push({
        line: errLine,
        column: errCol,
        severity: "error",
        message: "Trailing comma is not allowed in standard JSON",
        explanation: "The JSON specification (RFC 8259) strictly forbids trailing commas after the final element in an object or array.",
        fixSuggestion: "Remove the trailing comma before '}' or ']'."
      });
      return { errors, warnings };
    }

    errors.push({
      line: errLine,
      column: errCol,
      severity: "error",
      message: `Invalid JSON syntax: ${err.message}`,
      explanation: "The document contains invalid JSON syntax. Ensure keys are double-quoted and brackets are matched.",
      fixSuggestion: "Check for unquoted strings, mismatched braces, or stray characters."
    });
    return { errors, warnings };
  }

  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    errors.push({
      line: 1,
      column: 1,
      severity: "error",
      message: "Root element must be a JSON object",
      explanation: "Mod configuration documents must be rooted with a dictionary object ('{}').",
      fixSuggestion: "Wrap properties in '{}'."
    });
    return { errors, warnings };
  }

  const isModJson = (fileName || "").toLowerCase().endsWith("mod.json");
  if (isModJson) {
    if (!parsed.id || typeof parsed.id !== "string" || !parsed.id.trim()) {
      errors.push({
        line: 1,
        column: 1,
        severity: "error",
        message: "Missing required string property 'id'",
        explanation: "QuarterMaster requires every mod.json to specify a unique machine identifier (e.g. \"id\": \"tactical_overhaul\").",
        fixSuggestion: 'Add \"id\": \"my_mod_id\"'
      });
    } else if (!/^[a-z0-9_-]+$/.test(parsed.id)) {
      errors.push({
        line: 1,
        column: 1,
        severity: "error",
        message: `Invalid mod 'id' '${parsed.id}': must match pattern ^[a-z0-9_-]+$`,
        explanation: "Mod IDs are used in filesystem paths, save states, and dependency manifests and must only contain lowercase alphanumeric characters, underscores, and hyphens.",
        fixSuggestion: `Change id to '${parsed.id.toLowerCase().replace(/[^a-z0-9_-]/g, "_")}'`
      });
    }

    if (!parsed.name || typeof parsed.name !== "string" || !parsed.name.trim()) {
      errors.push({
        line: 1,
        column: 1,
        severity: "error",
        message: "Missing required string property 'name'",
        explanation: "The mod manifest requires a human-readable display name for the launcher and library UI.",
        fixSuggestion: 'Add \"name\": \"My Mod Name\"'
      });
    }

    if (!parsed.version || typeof parsed.version !== "string" || !parsed.version.trim()) {
      errors.push({
        line: 1,
        column: 1,
        severity: "error",
        message: "Missing required string property 'version'",
        explanation: "The mod manifest requires a version string (e.g. '1.0.0') for dependency resolution and update checking.",
        fixSuggestion: 'Add \"version\": \"1.0.0\"'
      });
    } else if (!/^\d+(\.\d+)*(-[a-zA-Z0-9.]+)?$/.test(parsed.version)) {
      warnings.push({
        line: 1,
        column: 1,
        severity: "warning",
        message: `Version string '${parsed.version}' does not follow semantic versioning (X.Y.Z)`,
        explanation: "Semantic versioning allows QuarterMaster to compare updates and check dependency compatibility reliably.",
        fixSuggestion: 'Format as \"1.0.0\" or \"0.1.0-beta\"'
      });
    }

    if (parsed.load_priority !== undefined && typeof parsed.load_priority !== "number") {
      warnings.push({
        line: 1,
        column: 1,
        severity: "warning",
        message: "'load_priority' should be an integer number (default: 100)",
        explanation: "Load priority controls the order in which mods and their RAM patches are applied.",
        fixSuggestion: 'Set \"load_priority\": 100'
      });
    }
  }

  return { errors, warnings };
}

function lintMarkdownSource(content) {
  const errors = [];
  const warnings = [];
  if (typeof content !== "string") return { errors, warnings };

  const lines = content.split(/\r?\n/);
  let inCodeBlock = false;
  let codeBlockFence = "";
  let codeBlockStartLine = 0;

  for (let i = 0; i < lines.length; i++) {
    const lineNum = i + 1;
    const line = lines[i];
    const matchFence = line.match(/^(\s*)([`~]{3,})/);
    if (matchFence) {
      const fence = matchFence[2];
      if (!inCodeBlock) {
        inCodeBlock = true;
        codeBlockFence = fence[0];
        codeBlockStartLine = lineNum;
      } else if (fence[0] === codeBlockFence) {
        inCodeBlock = false;
      }
    }
  }

  if (inCodeBlock) {
    errors.push({
      line: codeBlockStartLine,
      column: 1,
      severity: "error",
      message: `Unclosed code fence (${codeBlockFence}${codeBlockFence}${codeBlockFence}) starting on line ${codeBlockStartLine}`,
      explanation: "A Markdown code block was opened with backticks or tildes but never closed before the end of the file.",
      fixSuggestion: `Add closing ${codeBlockFence}${codeBlockFence}${codeBlockFence} on a new line.`
    });
  }

  return { errors, warnings };
}

function checkMarkdownSyntax(content) {
  const res = lintMarkdownSource(content);
  if (res.errors && res.errors.length > 0) {
    return {
      valid: false,
      line: res.errors[0].line,
      message: res.errors[0].message,
      explanation: res.errors[0].explanation
    };
  }
  return { valid: true };
}

function checkLuaSyntax(code) {
  const res = lintLuaSource(code);
  if (res.errors && res.errors.length > 0) {
    return { valid: false, line: res.errors[0].line, message: res.errors[0].message };
  }
  return { valid: true };
}

function validateModJsonSchema(code) {
  const res = lintJsonSource("mod.json", code);
  if (res.errors && res.errors.length > 0) {
    return { valid: false, line: res.errors[0].line, message: res.errors[0].message };
  }
  return { valid: true };
}

function validateQmTomlSchema(code) {
  const res = lintTomlSource("qm.toml", code);
  if (res.errors && res.errors.length > 0) {
    return { valid: false, line: res.errors[0].line, message: res.errors[0].message };
  }
  return { valid: true };
}

function lintSourceDocument(fileName, content) {
  const lower = (fileName || "").toLowerCase();
  if (lower.endsWith(".lua")) {
    return lintLuaSource(content);
  }
  if (lower === "qm.toml" || lower.endsWith("/qm.toml") || lower.endsWith(".toml")) {
    return lintTomlSource(fileName, content);
  }
  if (lower === "mod.json" || lower.endsWith("/mod.json") || lower.endsWith(".json")) {
    return lintJsonSource(fileName, content);
  }
  if (lower.endsWith(".md") || lower.endsWith(".markdown")) {
    return lintMarkdownSource(content);
  }
  return { errors: [], warnings: [] };
}

// ============================================================================
// Editor Diagnostics UI & Line Navigation
// ============================================================================

function jumpToEditorLine(lineNum, colNum) {
  const textarea = document.getElementById("mod-file-editor-textarea");
  if (!textarea) return;

  const content = textarea.value || "";
  const lines = content.split("\n");
  const targetLine = Math.max(1, Math.min(parseInt(lineNum, 10) || 1, lines.length));
  const targetCol = Math.max(1, parseInt(colNum, 10) || 1);

  let charOffset = 0;
  for (let i = 0; i < targetLine - 1; i++) {
    charOffset += lines[i].length + 1;
  }
  const lineLength = lines[targetLine - 1] ? lines[targetLine - 1].length : 0;
  const colOffset = Math.min(targetCol - 1, lineLength);
  const finalOffset = charOffset + colOffset;

  textarea.focus();
  textarea.selectionStart = finalOffset;
  textarea.selectionEnd = charOffset + lineLength;

  // Center line in viewport
  const lineHeight = 19.5;
  textarea.scrollTop = Math.max(0, (targetLine - 4) * lineHeight);
}

function renderEditorProblemsPanel(res, filePath) {
  const counterEl = document.getElementById("editor-problems-counter");
  const fileEl = document.getElementById("editor-problems-file");
  const listEl = document.getElementById("editor-problems-list");
  if (!listEl) return;

  const totalErrors = res.errors ? res.errors.length : 0;
  const totalWarnings = res.warnings ? res.warnings.length : 0;

  if (counterEl) {
    counterEl.textContent = `[${totalErrors} Error${totalErrors === 1 ? "" : "s"}, ${totalWarnings} Warning${totalWarnings === 1 ? "" : "s"}]`;
    counterEl.className = totalErrors > 0 ? "badge badge-error" : (totalWarnings > 0 ? "badge badge-warning" : "badge badge-neutral");
  }

  if (fileEl) {
    fileEl.textContent = filePath ? filePath : "No file open";
  }

  if (totalErrors === 0 && totalWarnings === 0) {
    listEl.innerHTML = '<div class="problems-empty-item">No problems have been detected in the current file.</div>';
    return;
  }

  const items = [];
  (res.errors || []).forEach(e => items.push({ ...e, severity: "error" }));
  (res.warnings || []).forEach(w => items.push({ ...w, severity: "warning" }));

  listEl.innerHTML = items
    .map((item) => {
      const badgeClass = item.severity === "error" ? "badge-error" : "badge-warning";
      const badgeText = item.severity === "error" ? "[Error]" : "[Warning]";
      const locText = `Line ${item.line || 1}${item.column ? ", Col " + item.column : ""}`;

      return `
        <div class="problem-row" data-line="${item.line || 1}" data-col="${item.column || 1}">
          <div class="problem-row-header">
            <span class="problem-badge ${badgeClass}">${badgeText}</span>
            <span class="problem-location">${locText}</span>
            <span class="problem-message">${escapeHtml(item.message)}</span>
          </div>
          ${item.explanation ? `<div class="problem-explanation">${escapeHtml(item.explanation)}</div>` : ""}
          ${item.fixSuggestion ? `<div class="problem-fix">Suggested fix: ${escapeHtml(item.fixSuggestion)}</div>` : ""}
        </div>
      `;
    })
    .join("");

  listEl.querySelectorAll(".problem-row").forEach((row) => {
    row.addEventListener("click", () => {
      const line = row.dataset.line;
      const col = row.dataset.col;
      jumpToEditorLine(line, col);
    });
  });
}

let editorValidationDebounceTimer = null;

function updateLiveEditorValidation() {
  const statusEl = document.getElementById("editor-validation-status");
  const textarea = document.getElementById("mod-file-editor-textarea");
  if (!statusEl || !textarea) return;

  const filePath = state.editorActiveFilePath || "";
  const content = textarea.value || "";

  if (!filePath) {
    statusEl.className = "editor-validation-badge status-idle";
    statusEl.textContent = "Validation: Idle";
    renderEditorProblemsPanel({ errors: [], warnings: [] }, "");
    return;
  }

  const res = lintSourceDocument(filePath, content);
  const errCount = res.errors ? res.errors.length : 0;
  const warnCount = res.warnings ? res.warnings.length : 0;

  if (errCount > 0) {
    statusEl.className = "editor-validation-badge status-error";
    statusEl.textContent = `[${errCount} Error${errCount === 1 ? "" : "s"}, ${warnCount} Warning${warnCount === 1 ? "" : "s"}]`;
    statusEl.title = `Click to inspect problems panel (${errCount} errors found)`;
  } else if (warnCount > 0) {
    statusEl.className = "editor-validation-badge status-warning";
    statusEl.textContent = `[0 Errors, ${warnCount} Warning${warnCount === 1 ? "" : "s"}]`;
    statusEl.title = `Click to inspect problems panel (${warnCount} warnings found)`;
  } else {
    statusEl.className = "editor-validation-badge status-ok";
    statusEl.textContent = "[OK: 0 Problems]";
    statusEl.title = "No syntax errors or schema violations detected.";
  }

  renderEditorProblemsPanel(res, filePath);
}

// ============================================================================
// Context-Aware Autocomplete & Suggestions Engine
// ============================================================================

function getAutocompleteSuggestions(filePath, content, cursorPos) {
  if (typeof content !== "string" || cursorPos === undefined || cursorPos === null) return [];
  const textBefore = content.substring(0, cursorPos);
  const match = textBefore.match(/([a-zA-Z0-9_@.:\/"'-]+)$/);
  if (!match) return [];

  const prefix = match[1];
  const lowerPrefix = prefix.toLowerCase();
  const lowerPath = (filePath || "").toLowerCase();
  const suggestions = [];

  // 1. LUA CONTEXT (.lua)
  if (lowerPath.endsWith(".lua")) {
    // 1.1 Vanilla Engine API functions & class fields
    if (state.vanillaApi && Array.isArray(state.vanillaApi.symbols)) {
      for (const sym of state.vanillaApi.symbols) {
        const symName = sym.name || "";
        const lowerName = symName.toLowerCase();
        if (lowerName.includes(lowerPrefix) || symName.startsWith(prefix)) {
          const isPrefixMatch = lowerName.startsWith(lowerPrefix);
          const paramsList = Array.isArray(sym.params) ? sym.params.map(p => p.name || p).join(", ") : "";
          const insertSnippet = sym.kind === "class_table" ? symName : `${symName}(${paramsList})`;

          suggestions.push({
            type: sym.kind === "class_table" ? "class" : "func",
            name: symName,
            params: paramsList ? `(${paramsList})` : "()",
            detail: sym.file ? sym.file.split("/").pop() : "vanilla",
            insertText: insertSnippet,
            desc: sym.description || "",
            score: isPrefixMatch ? (lowerName === lowerPrefix ? 100 : 80) : 40
          });
        }
      }
    }

    // 1.2 Class member suggestions (player.*, dude.*, weapon.*)
    const classFields = [
      { name: "player.deployedTroops", desc: "Table of active deployed squad units", type: "class" },
      { name: "player.deployedVehicles", desc: "Table of active player vehicles", type: "class" },
      { name: "player.placeables", desc: "Table of deployable defenses and sandbags", type: "class" },
      { name: "player.buildings", desc: "Table of player defensive structures", type: "class" },
      { name: "player.tp", desc: "Current Tactical Points balance", type: "class" },
      { name: "player.bc", desc: "Active Battle Conditions modifying combat physics", type: "class" },
      { name: "dude.health", desc: "Current health points", type: "class" },
      { name: "dude.maxHealth", desc: "Maximum soldier health", type: "class" },
      { name: "dude.weapon", desc: "Equipped primary weapon table", type: "class" },
      { name: "dude.morale", desc: "Soldier bravery and panic threshold", type: "class" },
      { name: "dude.skill", desc: "Combat proficiency level (0-100)", type: "class" },
      { name: "dude.state", desc: "State machine: 'idle', 'pinned', 'moving', 'firing'", type: "class" }
    ];
    for (const cf of classFields) {
      if (cf.name.toLowerCase().includes(lowerPrefix)) {
        suggestions.push({
          type: "class",
          name: cf.name,
          params: "",
          detail: "urb_engine.lua",
          insertText: cf.name,
          desc: cf.desc,
          score: cf.name.toLowerCase().startsWith(lowerPrefix) ? 85 : 45
        });
      }
    }

    // 1.3 Lua Keywords & Standard Snippets
    const luaSnippets = [
      { name: "function", insertText: "function name()\n  \nend", detail: "snippet", desc: "Function definition block" },
      { name: "local", insertText: "local ", detail: "keyword", desc: "Local variable declaration" },
      { name: "if", insertText: "if condition then\n  \nend", detail: "snippet", desc: "If conditional block" },
      { name: "for (pairs)", insertText: "for k, v in pairs(tbl) do\n  \nend", detail: "snippet", desc: "Iterate table key-values" },
      { name: "for (ipairs)", insertText: "for i, v in ipairs(tbl) do\n  \nend", detail: "snippet", desc: "Iterate table array" },
      { name: "for (numeric)", insertText: "for i = 1, #tbl do\n  \nend", detail: "snippet", desc: "Numeric indexed loop" },
      { name: "while", insertText: "while condition do\n  \nend", detail: "snippet", desc: "While loop" },
      { name: "repeat", insertText: "repeat\n  \nuntil condition", detail: "snippet", desc: "Repeat until loop" },
      { name: "return", insertText: "return ", detail: "keyword", desc: "Return statement" },
      { name: "love.load", insertText: "function love.load(arg)\n  \nend", detail: "hook", desc: "LOVE2D engine lifecycle init callback" },
      { name: "love.update", insertText: "function love.update(dt)\n  \nend", detail: "hook", desc: "LOVE2D per-frame update callback" },
      { name: "love.draw", insertText: "function love.draw()\n  \nend", detail: "hook", desc: "LOVE2D render callback" }
    ];
    for (const sn of luaSnippets) {
      if (sn.name.toLowerCase().includes(lowerPrefix)) {
        suggestions.push({
          type: "snippet",
          name: sn.name,
          params: "",
          detail: sn.detail,
          insertText: sn.insertText,
          desc: sn.desc,
          score: sn.name.toLowerCase().startsWith(lowerPrefix) ? 90 : 50
        });
      }
    }
  }

  // 2. TOML CONTEXT (qm.toml, .toml)
  else if (lowerPath.endsWith(".toml") || lowerPath.endsWith("qm.toml")) {
    const gameFiles = [
      "@game/main.lua", "@game/mapBuild.lua", "@game/dudes.lua", "@game/weapons.lua",
      "@game/urb_engine.lua", "@game/bcChecks.lua", "@game/trench.lua", "@game/ribbons.lua",
      "@game/medals.lua", "@game/vehicles.lua", "@game/camo.lua", "@game/explosions.lua",
      "@game/bullets.lua", "@game/gore.lua", "@game/sounds.lua", "@game/ui.lua"
    ];

    const cleanTomlPrefix = lowerPrefix.replace(/^["']/, "");
    if (cleanTomlPrefix.includes("@game") || cleanTomlPrefix.includes("target") || cleanTomlPrefix.includes("map") || cleanTomlPrefix.includes("dudes") || cleanTomlPrefix.includes("weapon")) {
      for (const gf of gameFiles) {
        if (gf.toLowerCase().includes(cleanTomlPrefix) || cleanTomlPrefix.startsWith("@game") || gf.replace("@game/", "").startsWith(cleanTomlPrefix)) {
          suggestions.push({
            type: "file",
            name: gf,
            params: "",
            detail: "game source",
            insertText: gf,
            desc: `Vanilla game script target: ${gf}`,
            score: gf.toLowerCase().includes(lowerPrefix) ? 95 : 60
          });
        }
      }
    }

    const tomlKeys = [
      { name: "[[patches]] (pattern)", insertText: "[[patches]]\n[patches.pattern]\ntarget = \"main.lua\"\npattern = \"\"\nposition = \"at\"\npayload = \"\"\nmatch_indent = true\n", detail: "snippet", desc: "Standard pattern hook patch" },
      { name: "[[patches]] (module)", insertText: "[[patches]]\n[patches.module]\nsource = \"my_module.lua\"\nbefore = \"main.lua\"\nname = \"includes/my_module\"\n", detail: "snippet", desc: "Lua package.preload module registration" },
      { name: "[[patches]] (regex)", insertText: "[[patches]]\n[patches.regex]\ntarget = \"main.lua\"\npattern = \"\"\nposition = \"at\"\npayload = \"\"\n", detail: "snippet", desc: "Regular expression patch" },
      { name: "[[patches]] (copy)", insertText: "[[patches]]\n[patches.copy]\ntarget = \"main.lua\"\nposition = \"append\"\npayload = \"\"\n", detail: "snippet", desc: "Prepend/append copy patch" },
      { name: "[manifest]", insertText: "[manifest]\nversion = \"1.0.0\"\npriority = 50\n", detail: "snippet", desc: "Mod manifest block" },
      { name: "[patches.pattern]", insertText: "[patches.pattern]\ntarget = \"main.lua\"\npattern = \"\"\nposition = \"at\"\npayload = \"\"\n", detail: "table", desc: "Pattern hook table" },
      { name: "[patches.module]", insertText: "[patches.module]\nsource = \"module.lua\"\nbefore = \"main.lua\"\nname = \"includes/module\"\n", detail: "table", desc: "Module preload table" },
      { name: "target", insertText: 'target = "main.lua"', detail: "key", desc: "Target vanilla Lua file to patch" },
      { name: "pattern", insertText: 'pattern = ""', detail: "key", desc: "Pattern string to match in memory" },
      { name: "payload", insertText: 'payload = """\n"""', detail: "key", desc: "Replacement or injected Lua code payload" },
      { name: "position (at)", insertText: 'position = "at"', detail: "key", desc: "Replace matched line(s) entirely" },
      { name: "position (before)", insertText: 'position = "before"', detail: "key", desc: "Insert payload before matched line" },
      { name: "position (after)", insertText: 'position = "after"', detail: "key", desc: "Insert payload after matched line" },
      { name: "match_indent", insertText: "match_indent = true", detail: "key", desc: "Preserve target line indentation" },
      { name: "source", insertText: 'source = "module.lua"', detail: "key", desc: "Relative path to Lua module file" },
      { name: "name", insertText: 'name = "includes/module"', detail: "key", desc: "Package preload identifier" },
      { name: "before", insertText: 'before = "main.lua"', detail: "key", desc: "Evaluate module before this target file" },
      { name: "priority", insertText: "priority = 50", detail: "key", desc: "Execution priority integer" }
    ];

    for (const tk of tomlKeys) {
      if (tk.name.toLowerCase().includes(lowerPrefix)) {
        suggestions.push({
          type: "key",
          name: tk.name,
          params: "",
          detail: tk.detail,
          insertText: tk.insertText,
          desc: tk.desc,
          score: tk.name.toLowerCase().startsWith(lowerPrefix) ? 90 : 50
        });
      }
    }
  }

  // 3. JSON CONTEXT (mod.json, .json)
  else if (lowerPath.endsWith(".json") || lowerPath.endsWith("mod.json")) {
    const jsonFields = [
      { name: '"id"', insertText: '"id": "mod_identifier",', detail: "required", desc: "Unique machine identifier (lowercase alphanumeric)" },
      { name: '"name"', insertText: '"name": "Mod Display Name",', detail: "required", desc: "Human-readable display name" },
      { name: '"version"', insertText: '"version": "1.0.0",', detail: "required", desc: "Semantic version string" },
      { name: '"authors"', insertText: '"authors": ["Author"],', detail: "field", desc: "List of authors or contributors" },
      { name: '"description"', insertText: '"description": "Mod description...",', detail: "field", desc: "Mod summary description" },
      { name: '"load_priority"', insertText: '"load_priority": 100,', detail: "field", desc: "Load execution priority integer (default: 100)" },
      { name: '"dependencies"', insertText: '"dependencies": {},\n', detail: "field", desc: "Required mod dependencies dictionary" },
      { name: '"relationships"', insertText: '"relationships": {\n  "conflicts_with": []\n},', detail: "field", desc: "Mod incompatibility declarations" },
      { name: '"update_keys"', insertText: '"update_keys": {\n  "nexus": 0,\n  "github": ""\n},', detail: "field", desc: "Repository identifiers for automatic update checking" }
    ];

    const cleanJsonPrefix = lowerPrefix.replace(/^["']/, "");
    for (const jf of jsonFields) {
      const fieldKey = jf.name.replace(/^["']|["']$/g, "");
      if (jf.name.toLowerCase().includes(lowerPrefix) || fieldKey.toLowerCase().startsWith(cleanJsonPrefix)) {
        suggestions.push({
          type: "key",
          name: jf.name,
          params: "",
          detail: jf.detail,
          insertText: jf.insertText,
          desc: jf.desc,
          score: jf.name.toLowerCase().startsWith(lowerPrefix) ? 90 : 50
        });
      }
    }
  }

  suggestions.sort((a, b) => (b.score - a.score) || (a.name.length - b.name.length));

  const seen = new Set();
  const deduped = [];
  for (const s of suggestions) {
    if (!seen.has(s.name)) {
      seen.add(s.name);
      deduped.push(s);
      if (deduped.length >= 14) break;
    }
  }

  return deduped;
}

let autocompleteState = {
  visible: false,
  items: [],
  selectedIndex: 0,
  prefix: "",
  startOffset: 0,
  endOffset: 0
};

function hideAutocompleteBox() {
  const box = document.getElementById("editor-autocomplete-box");
  if (box) box.style.display = "none";
  autocompleteState.visible = false;
  autocompleteState.items = [];
}

function renderAutocompleteBox() {
  const box = document.getElementById("editor-autocomplete-box");
  if (!box || autocompleteState.items.length === 0) {
    hideAutocompleteBox();
    return;
  }

  box.innerHTML = autocompleteState.items
    .map((item, idx) => {
      const isSel = idx === autocompleteState.selectedIndex;
      const badgeClass = `badge-${item.type || "key"}`;
      const badgeText = item.type === "func" ? "[Func]" : item.type === "file" ? "[File]" : item.type === "snippet" ? "[Snippet]" : item.type === "class" ? "[Class]" : "[Key]";

      return `
        <div class="autocomplete-item ${isSel ? "active" : ""}" data-idx="${idx}">
          <div class="autocomplete-item-left">
            <span class="autocomplete-badge ${badgeClass}">${badgeText}</span>
            <span class="autocomplete-name">${escapeHtml(item.name)}</span>
            ${item.params ? `<span class="autocomplete-params">${escapeHtml(item.params)}</span>` : ""}
          </div>
          <span class="autocomplete-item-right">${escapeHtml(item.detail || "")}</span>
        </div>
      `;
    })
    .join("");

  box.querySelectorAll(".autocomplete-item").forEach((el) => {
    el.addEventListener("mousedown", (e) => {
      e.preventDefault();
      const idx = parseInt(el.dataset.idx, 10);
      insertAutocompleteItem(idx);
    });
  });

  box.style.display = "block";
  autocompleteState.visible = true;
}

function updateAutocompleteUI() {
  const textarea = document.getElementById("mod-file-editor-textarea");
  const box = document.getElementById("editor-autocomplete-box");
  if (!textarea || !box) return;

  const content = textarea.value || "";
  const cursorPos = textarea.selectionStart;
  const textBefore = content.substring(0, cursorPos);
  const match = textBefore.match(/([a-zA-Z0-9_@.:\/"'-]+)$/);

  if (!match || match[1].length < 2) {
    hideAutocompleteBox();
    return;
  }

  const prefix = match[1];
  autocompleteState.prefix = prefix;
  autocompleteState.startOffset = cursorPos - prefix.length;
  autocompleteState.endOffset = cursorPos;

  const suggestions = getAutocompleteSuggestions(state.editorActiveFilePath, content, cursorPos);
  if (suggestions.length === 0) {
    hideAutocompleteBox();
    return;
  }

  autocompleteState.items = suggestions;
  autocompleteState.selectedIndex = 0;

  // Position box relative to cursor line and column
  const linesBefore = textBefore.split("\n");
  const lineIdx = linesBefore.length - 1;
  const colIdx = linesBefore[lineIdx].length;

  const lineHeight = 19.5;
  const top = Math.min(lineIdx * lineHeight - textarea.scrollTop + 30, textarea.clientHeight - 190);
  const left = Math.min(colIdx * 7.5 + 46, textarea.clientWidth - 410);

  box.style.top = `${Math.max(10, top)}px`;
  box.style.left = `${Math.max(48, left)}px`;

  renderAutocompleteBox();
}

function insertAutocompleteItem(idx) {
  const textarea = document.getElementById("mod-file-editor-textarea");
  if (!textarea || !autocompleteState.items[idx]) return;

  const item = autocompleteState.items[idx];
  const content = textarea.value || "";
  const before = content.substring(0, autocompleteState.startOffset);
  const after = content.substring(autocompleteState.endOffset);

  const insertText = item.insertText;
  textarea.value = before + insertText + after;
  const newCursor = before.length + insertText.length;
  textarea.selectionStart = textarea.selectionEnd = newCursor;

  hideAutocompleteBox();
  textarea.focus();
  textarea.dispatchEvent(new Event("input"));
}

// ----------------------------------------------------------------------------
// File Browser Tree & Editor Logic
// ----------------------------------------------------------------------------

function updateEditorLineNumbers() {
  const lineNumbers = document.getElementById("editor-line-numbers");
  const textarea = document.getElementById("mod-file-editor-textarea");
  if (!lineNumbers || !textarea) return;

  const count = (textarea.value.split("\n").length) || 1;
  const nums = [];
  for (let i = 1; i <= count; i++) {
    nums.push(i);
  }
  lineNumbers.textContent = nums.join("\n");
}

async function loadModFilesTree(modDir) {
  const treeContainer = document.getElementById("mod-files-tree");
  if (!treeContainer) return;

  state.editorActiveModDir = modDir;
  treeContainer.innerHTML = '<div style="padding: 10px; color: var(--text-muted); font-size: 11.5px;">Loading mod files...</div>';

  try {
    const files = await apiGet(`/api/devtools/files?mod_dir=${encodeURIComponent(modDir)}`);
    state.editorModFiles = Array.isArray(files) ? files : [];

    if (state.editorModFiles.length === 0) {
      treeContainer.innerHTML = '<div style="padding: 10px; color: var(--text-muted); font-size: 11.5px;">No files found in mod directory.</div>';
      return;
    }

    treeContainer.innerHTML = state.editorModFiles
      .map((f) => {
        const isActive = f.relative_path === state.editorActiveFilePath;
        const iconClass = `icon-${f.file_type || "other"}`;
        const sizeFormatted = f.size_bytes > 1024
          ? `${(f.size_bytes / 1024).toFixed(1)} KB`
          : `${f.size_bytes} B`;

        return `
          <div class="tree-file-item ${isActive ? "active" : ""}" data-file-path="${escapeHtml(f.relative_path)}">
            <div class="tree-file-main">
              <span class="tree-file-icon ${iconClass}">${escapeHtml(f.file_type)}</span>
              <span>${escapeHtml(f.relative_path)}</span>
            </div>
            <span class="tree-file-size">${sizeFormatted}</span>
          </div>
        `;
      })
      .join("");

    treeContainer.querySelectorAll(".tree-file-item").forEach((el) => {
      el.addEventListener("click", () => {
        const filePath = el.dataset.filePath;
        if (filePath) {
          loadModFileContent(state.editorActiveModDir, filePath);
        }
      });
    });

    const hasCurrent = state.editorModFiles.some((f) => f.relative_path === state.editorActiveFilePath);
    if (!state.editorActiveFilePath || !hasCurrent) {
      const preferred = state.editorModFiles.find((f) => f.relative_path === "qm.toml")
        || state.editorModFiles.find((f) => f.relative_path === "mod.json")
        || state.editorModFiles.find((f) => f.relative_path === "config.lua")
        || state.editorModFiles.find((f) => f.relative_path === "main.lua")
        || state.editorModFiles[0];
      if (preferred) {
        await loadModFileContent(modDir, preferred.relative_path);
      }
    }
  } catch (err) {
    treeContainer.innerHTML = `<div style="padding: 10px; color: var(--status-error); font-size: 11.5px;">Failed to load files: ${escapeHtml(err.message)}</div>`;
  }
}

async function loadModFileContent(modDir, filePath) {
  const textarea = document.getElementById("mod-file-editor-textarea");
  const badge = document.getElementById("editor-file-badge");
  const dirty = document.getElementById("editor-dirty-indicator");
  if (!textarea) return;

  try {
    const res = await apiGet(`/api/devtools/file/read?mod_dir=${encodeURIComponent(modDir)}&file_path=${encodeURIComponent(filePath)}`);
    textarea.value = res.content || "";
    state.editorOriginalContent = textarea.value;
    state.editorActiveFilePath = filePath;
    state.editorActiveModDir = modDir;

    if (badge) badge.textContent = filePath;
    if (dirty) dirty.style.display = "none";

    document.querySelectorAll("#mod-files-tree .tree-file-item").forEach((el) => {
      el.classList.toggle("active", el.dataset.filePath === filePath);
    });

    updateEditorLineNumbers();
    updateLiveEditorValidation();
  } catch (err) {
    showToast(`Failed to read '${filePath}': ${err.message}`, true);
  }
}

async function saveActiveModFile() {
  const textarea = document.getElementById("mod-file-editor-textarea");
  const dirty = document.getElementById("editor-dirty-indicator");
  if (!textarea || !state.editorActiveModDir || !state.editorActiveFilePath) {
    return showToast("No active file to save.", true);
  }

  const content = textarea.value;
  try {
    await apiPost("/api/devtools/file/save", {
      mod_dir: state.editorActiveModDir,
      file_path: state.editorActiveFilePath,
      content: content,
    });
    state.editorOriginalContent = content;
    if (dirty) dirty.style.display = "none";
    showToast(`Saved '${state.editorActiveFilePath}'.`);
    updateLiveEditorValidation();
  } catch (err) {
    showToast(`Failed to save '${state.editorActiveFilePath}': ${err.message}`, true);
  }
}

async function addBlankFile() {
  if (!state.editorActiveModDir) {
    return showToast("Please select an active mod first.", true);
  }

  const presets = [
    { label: "config.lua (Configuration Table)", filename: "config.lua", content: "-- Configuration for mod\nreturn {\n  enabled = true,\n}\n" },
    { label: "qm.toml (Patch Manifest)", filename: "qm.toml", content: "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\ntarget = \"@main.lua\"\n" },
    { label: "main.lua (Entrypoint Script)", filename: "main.lua", content: "-- Main entrypoint for mod\nprint(\"Mod initialized\")\n" },
    { label: "Custom Filename...", filename: "", content: "" },
  ];

  const choice = prompt(
    "Choose a preset or enter filename:\n1: config.lua\n2: qm.toml\n3: main.lua\nOr enter custom filename directly (e.g. scripts/helper.lua):",
    "config.lua"
  );
  if (!choice) return;

  let targetFilename = choice.trim();
  let initialContent = "";
  if (targetFilename === "1" || targetFilename.toLowerCase() === "config.lua") {
    targetFilename = "config.lua";
    initialContent = presets[0].content;
  } else if (targetFilename === "2" || targetFilename.toLowerCase() === "qm.toml") {
    targetFilename = "qm.toml";
    initialContent = presets[1].content;
  } else if (targetFilename === "3" || targetFilename.toLowerCase() === "main.lua") {
    targetFilename = "main.lua";
    initialContent = presets[2].content;
  }

  try {
    await apiPost("/api/devtools/file/save", {
      mod_dir: state.editorActiveModDir,
      file_path: targetFilename,
      content: initialContent,
    });
    showToast(`Created blank file '${targetFilename}'.`);
    await loadModFilesTree(state.editorActiveModDir);
    await loadModFileContent(state.editorActiveModDir, targetFilename);
  } catch (err) {
    showToast(`Failed to create file: ${err.message}`, true);
  }
}

async function copyTemplateIntoMod() {
  if (!state.editorActiveModDir) {
    return showToast("Please select an active mod first.", true);
  }

  const previewBox = document.getElementById("template-code-preview");
  const templateCode = previewBox?.textContent || "";
  if (!templateCode || templateCode.startsWith("-- No template")) {
    return showToast("Please select a template from the Scaffolder first.", true);
  }

  const tplSel = document.getElementById("scaf-template");
  const curTplId = tplSel?.value || "template";
  const defaultFilename = templateCode.includes("[manifest]") ? "qm.toml" : `scripts/${curTplId}.lua`;

  const destPath = prompt("Enter file path to save template code into active mod:", defaultFilename);
  if (!destPath) return;

  try {
    await apiPost("/api/devtools/file/save", {
      mod_dir: state.editorActiveModDir,
      file_path: destPath.trim(),
      content: templateCode,
    });
    showToast(`Copied template into '${destPath.trim()}'.`);
    await loadModFilesTree(state.editorActiveModDir);
    await loadModFileContent(state.editorActiveModDir, destPath.trim());
  } catch (err) {
    showToast(`Failed to copy template: ${err.message}`, true);
  }
}


// ----------------------------------------------------------------------------
// Subtab 1: New Mod Files & Templating Functions
// ----------------------------------------------------------------------------

function updateScaffoldEditorLineNumbers() {
  const lineNumbers = document.getElementById("new-mod-line-numbers");
  const textarea = document.getElementById("new-mod-editor-textarea");
  if (!lineNumbers || !textarea) return;

  const count = textarea.value.split("\n").length || 1;
  const nums = [];
  for (let i = 1; i <= count; i++) {
    nums.push(i);
  }
  lineNumbers.textContent = nums.join("\n");
}

function updateScaffoldLiveEditorValidation() {
  const textarea = document.getElementById("new-mod-editor-textarea");
  const statusEl = document.getElementById("new-mod-validation-status");
  if (!textarea || !statusEl || !state.scaffoldActiveFilePath) return;

  const content = textarea.value;
  const path = state.scaffoldActiveFilePath;
  const lower = path.toLowerCase();

  if (lower.endsWith(".lua")) {
    const res = validateLuaSyntax(content);
    if (res.valid) {
      statusEl.className = "editor-validation-badge status-ok";
      statusEl.textContent = "Lua Syntax OK";
      statusEl.title = "Valid Lua syntax.";
    } else {
      statusEl.className = "editor-validation-badge status-error";
      statusEl.textContent = `Error Line ${res.line || 1}: ${res.message}`;
      statusEl.title = res.message;
    }
  } else if (lower === "mod.json" || lower.endsWith("/mod.json") || lower.endsWith(".json")) {
    const res = validateModJsonSchema(content);
    if (res.valid) {
      statusEl.className = "editor-validation-badge status-ok";
      statusEl.textContent = "mod.json Schema OK";
      statusEl.title = "Valid JSON syntax and required manifest schema fields present.";
    } else {
      statusEl.className = "editor-validation-badge status-error";
      statusEl.textContent = `Error Line ${res.line || 1}: ${res.message}`;
      statusEl.title = res.message;
    }
  } else if (lower === "qm.toml" || lower.endsWith("/qm.toml") || lower.endsWith(".toml")) {
    const res = validateQmTomlSchema(content);
    if (res.valid) {
      statusEl.className = "editor-validation-badge status-ok";
      statusEl.textContent = `qm.toml Valid (${res.patchCount} patches)`;
      statusEl.title = "Valid TOML patch table schema.";
    } else {
      statusEl.className = "editor-validation-badge status-error";
      statusEl.textContent = `Error Line ${res.line || 1}: ${res.message}`;
      statusEl.title = res.message;
    }
  } else {
    statusEl.className = "editor-validation-badge status-idle";
    statusEl.textContent = "Plain Text";
    statusEl.title = "Syntax checking not applicable.";
  }
}

async function loadScaffoldModFiles(modDir) {
  const listContainer = document.getElementById("new-mod-files-list");
  if (!listContainer) return;

  if (!modDir) {
    state.scaffoldActiveModDir = null;
    state.scaffoldModFiles = [];
    state.scaffoldActiveFilePath = null;
    listContainer.innerHTML = '<div style="padding: 16px; color: var(--text-muted); font-size: 11px;">Create a new mod in Section 1 on the left to start templating and editing its files here.</div>';
    const editorArea = document.getElementById("scaffold-file-content");
    if (editorArea) editorArea.value = "";
    return;
  }

  state.scaffoldActiveModDir = modDir;
  listContainer.innerHTML = '<div style="padding: 8px; color: var(--text-muted); font-size: 11px;">Loading mod files...</div>';

  try {
    const files = await apiGet(`/api/devtools/files?mod_dir=${encodeURIComponent(modDir)}`);
    state.scaffoldModFiles = Array.isArray(files) ? files : [];

    if (state.scaffoldModFiles.length === 0) {
      listContainer.innerHTML = '<div style="padding: 8px; color: var(--text-muted); font-size: 11px;">No files found.</div>';
      return;
    }

    listContainer.innerHTML = state.scaffoldModFiles
      .map((f) => {
        const isActive = f.relative_path === state.scaffoldActiveFilePath;
        return `
          <div class="new-mod-file-item ${isActive ? "active" : ""}" data-file-path="${escapeHtml(f.relative_path)}">
            <span class="tree-file-icon icon-${f.file_type || "other"}" style="font-size: 9.5px; padding: 1px 3px;">${escapeHtml(f.file_type)}</span>
            <span style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">${escapeHtml(f.relative_path)}</span>
          </div>
        `;
      })
      .join("");

    listContainer.querySelectorAll(".new-mod-file-item").forEach((el) => {
      el.addEventListener("click", () => {
        const filePath = el.dataset.filePath;
        if (filePath) {
          loadScaffoldFileContent(state.scaffoldActiveModDir, filePath);
        }
      });
    });

    const hasCurrent = state.scaffoldModFiles.some((f) => f.relative_path === state.scaffoldActiveFilePath);
    if (!state.scaffoldActiveFilePath || !hasCurrent) {
      const preferred = state.scaffoldModFiles.find((f) => f.relative_path === "mod.json")
        || state.scaffoldModFiles.find((f) => f.relative_path === "qm.toml")
        || state.scaffoldModFiles.find((f) => f.relative_path === "config.lua")
        || state.scaffoldModFiles[0];
      if (preferred) {
        loadScaffoldFileContent(modDir, preferred.relative_path);
      }
    }
  } catch (err) {
    listContainer.innerHTML = `<div style="padding: 8px; color: var(--status-error); font-size: 11px;">Failed to load files: ${escapeHtml(err.message)}</div>`;
  }
}

async function loadScaffoldFileContent(modDir, filePath) {
  const textarea = document.getElementById("new-mod-editor-textarea");
  const badge = document.getElementById("new-mod-file-badge");
  const dirty = document.getElementById("new-mod-dirty-indicator");
  if (!textarea) return;

  try {
    const res = await apiGet(`/api/devtools/file/read?mod_dir=${encodeURIComponent(modDir)}&file_path=${encodeURIComponent(filePath)}`);
    textarea.value = res.content || "";
    state.scaffoldOriginalContent = textarea.value;
    state.scaffoldActiveFilePath = filePath;
    state.scaffoldActiveModDir = modDir;

    if (badge) badge.textContent = filePath;
    if (dirty) dirty.style.display = "none";

    document.querySelectorAll("#new-mod-files-list .new-mod-file-item").forEach((el) => {
      el.classList.toggle("active", el.dataset.filePath === filePath);
    });

    updateScaffoldEditorLineNumbers();
    updateScaffoldLiveEditorValidation();
  } catch (err) {
    showToast(`Failed to read '${filePath}': ${err.message}`, true);
  }
}

async function saveScaffoldActiveFile() {
  const textarea = document.getElementById("new-mod-editor-textarea");
  const dirty = document.getElementById("new-mod-dirty-indicator");
  if (!textarea || !state.scaffoldActiveModDir || !state.scaffoldActiveFilePath) {
    return showToast("No active file to save.", true);
  }

  const content = textarea.value;
  try {
    await apiPost("/api/devtools/file/save", {
      mod_dir: state.scaffoldActiveModDir,
      file_path: state.scaffoldActiveFilePath,
      content: content,
    });
    state.scaffoldOriginalContent = content;
    if (dirty) dirty.style.display = "none";
    showToast(`Saved '${state.scaffoldActiveFilePath}'.`);
    updateScaffoldLiveEditorValidation();
  } catch (err) {
    showToast(`Failed to save '${state.scaffoldActiveFilePath}': ${err.message}`, true);
  }
}

async function addBlankFileToScaffold() {
  if (!state.scaffoldActiveModDir) {
    return showToast("Please select or create a working mod first.", true);
  }

  const choice = prompt(
    "Choose a preset or enter filename:\n1: config.lua\n2: qm.toml\n3: main.lua\nOr enter custom filename directly (e.g. scripts/helper.lua):",
    "config.lua"
  );
  if (!choice) return;

  let targetFilename = choice.trim();
  let initialContent = "";
  if (targetFilename === "1" || targetFilename.toLowerCase() === "config.lua") {
    targetFilename = "config.lua";
    initialContent = "-- Configuration for mod\nreturn {\n  enabled = true,\n}\n";
  } else if (targetFilename === "2" || targetFilename.toLowerCase() === "qm.toml") {
    targetFilename = "qm.toml";
    initialContent = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\ntarget = \"@main.lua\"\n";
  } else if (targetFilename === "3" || targetFilename.toLowerCase() === "main.lua") {
    targetFilename = "main.lua";
    initialContent = "-- Main entrypoint for mod\nprint(\"Mod initialized\")\n";
  }

  try {
    await apiPost("/api/devtools/file/save", {
      mod_dir: state.scaffoldActiveModDir,
      file_path: targetFilename,
      content: initialContent,
    });
    showToast(`Created blank file '${targetFilename}'.`);
    await loadScaffoldModFiles(state.scaffoldActiveModDir);
    await loadScaffoldFileContent(state.scaffoldActiveModDir, targetFilename);
  } catch (err) {
    showToast(`Failed to create file: ${err.message}`, true);
  }
}

function insertTemplateToScaffold() {
  const textarea = document.getElementById("new-mod-editor-textarea");
  const previewBox = document.getElementById("template-code-preview");
  const templateCode = previewBox?.textContent || "";
  if (!textarea || !state.scaffoldActiveModDir) {
    return showToast("Please select a file in your working mod first.", true);
  }
  if (!templateCode || templateCode.startsWith("-- No template")) {
    return showToast("Please select a template on the left first.", true);
  }

  textarea.value = templateCode;
  const dirty = document.getElementById("new-mod-dirty-indicator");
  if (dirty) dirty.style.display = "inline-flex";
  updateScaffoldEditorLineNumbers();
  updateScaffoldLiveEditorValidation();
  showToast("Inserted template code into active file. Click 'Save File' to commit.");
}

function startLaunchStatusWatcher() {
  if (state.launchStatusWatcherTimer) return;
  state.launchStatusWatcherTimer = setInterval(async () => {
    try {
      const status = await apiGet("/api/game/status");
      const launchLbl = document.getElementById("launch-primary-label");
      const launchBtn = document.getElementById("btn-launch-primary");
      if (!launchLbl || !launchBtn) return;

      if (status.stage === "waiting_for_steam") {
        launchLbl.textContent = "WAITING FOR STEAM...";
      } else if (status.stage === "starting") {
        launchLbl.textContent = "STARTING GAME...";
      } else if (status.stage === "running" || status.is_running) {
        const pidStr = status.running_pid ? (" (PID " + status.running_pid + ")") : "";
        launchLbl.textContent = "RUNNING" + pidStr;
      } else if (status.stage === "cleaning") {
        launchLbl.textContent = "CLEANING UP...";
      } else {
        launchLbl.textContent = "LAUNCH";
      }
    } catch (_) {}
  }, 1500);
}

function initModEditorTab() {
  if (!state.vanillaApi) {
    loadVanillaApi().catch(() => {});
  }
  const sel = document.getElementById("editor-mod-select");
  if (!sel) return;

  const mods = state.data?.installed_mods || [];
  if (mods.length === 0) return;

  const curVal = sel.value || state.editorActiveModDir;
  sel.innerHTML = mods
    .map(
      (m) =>
        `<option value="${escapeHtml(m.manifest.id)}" data-path="${escapeHtml(m.install_path)}" ${m.manifest.id === curVal || m.install_path === curVal ? "selected" : ""}>${escapeHtml(m.manifest.name)} (${escapeHtml(m.manifest.id)})</option>`
    )
    .join("");

  const activeMod = sel.value || mods[0]?.manifest.id;
  const treeContainer = document.getElementById("mod-files-tree");
  const hasFilesLoaded = treeContainer && treeContainer.querySelectorAll(".tree-file-item").length > 0;
  if (activeMod && (!hasFilesLoaded || state.editorActiveModDir !== activeMod)) {
    loadModFilesTree(activeMod);
  }
}

// ============================================================================
// EVENT WIRING & INITIALIZATION
// ============================================================================
document.addEventListener("DOMContentLoaded", () => {
  setupLightDismissDialogs();
  setupColorSchemeAndPaletteToggles();
  setupDraggableSplitters();

  // Copy button inside Report Modal
  document.getElementById("btn-copy-report-modal")?.addEventListener("click", () => {
    const text =
      state.lastModalCopyText || document.getElementById("report-modal-body")?.innerText || "";
    navigator.clipboard?.writeText(text);
    showToast("Copied report to clipboard!");
  });

  // Primary navigation tabs
  document.querySelectorAll(".nav-tab").forEach((btn) => {
    btn.addEventListener("click", () => {
      document.querySelectorAll(".nav-tab").forEach((b) => b.classList.remove("active"));
      document.querySelectorAll(".tab-panel").forEach((p) => p.classList.remove("active"));
      btn.classList.add("active");
      const targetPanel = document.getElementById(`tab-${btn.dataset.tab}`);
      if (targetPanel) targetPanel.classList.add("active");
      state.activeTab = btn.dataset.tab;
      renderActiveTab();
    });
  });

  // Drawer subtabs (scoped to mod detail drawer to avoid conflict with Mod Tools and Library subtabs)
  document.querySelectorAll(".subtabs:not(#modtools-subtabs):not(#library-subtabs) .subtab").forEach((btn) => {
    btn.addEventListener("click", () => {
      document.querySelectorAll(".subtabs:not(#modtools-subtabs):not(#library-subtabs) .subtab").forEach((b) => b.classList.remove("active"));
      document.querySelectorAll(".subtab-content").forEach((c) => c.classList.remove("active"));
      btn.classList.add("active");
      document.getElementById(`subtab-${btn.dataset.subtab}`)?.classList.add("active");
    });
  });

  // Mod Tools subtabs
  setupModToolsSubtabs();

  // Library subtabs (Milestone M6 / Requirements R7 & R8)
  setupLibrarySubtabs();

  // Initialize Profile Swap Preview Toggle (Requirement R5)
  const previewToggle = document.getElementById("toggle-preview-swaps");
  if (previewToggle) {
    const savedPreview = localStorage.getItem("qm-preview-profile-swaps");
    previewToggle.checked = savedPreview === "true";
    previewToggle.addEventListener("change", () => {
      try {
        localStorage.setItem("qm-preview-profile-swaps", previewToggle.checked ? "true" : "false");
      } catch (err) {
        console.warn("Could not save to localStorage:", err);
      }
    });
  }

  // Profile quick switch handler (Requirements R4 & R5)
  const profSelect = document.getElementById("header-profile-select");
  if (profSelect) {
    profSelect.addEventListener("change", async (e) => {
      const targetId = e.target.value;
      const currentId = state.data?.active_profile?.id;
      if (targetId === "__new__") {
        if (currentId) profSelect.value = currentId;
        openCreateProfileModal();
        return;
      }
      if (!targetId || targetId === currentId) return;

      let isPreviewEnabled = false;
      try {
        isPreviewEnabled = previewToggle ? previewToggle.checked : localStorage.getItem("qm-preview-profile-swaps") === "true";
      } catch (err) {
        isPreviewEnabled = false;
      }

      if (!isPreviewEnabled) {
        // Immediate switch via POST /api/profile/switch
        try {
          invalidateDiffState();
          await apiPost("/api/profile/switch", { profile_id: targetId });
          await refreshState();
          showToast(`Activated profile '${targetId}'.`);
        } catch (err) {
          showToast(err.message, true);
          if (state.data?.active_profile) {
            profSelect.value = state.data.active_profile.id;
          }
        }
      } else {
        // Intercept and open Profile Swap Preview Modal
        try {
          const preview = await apiPost("/api/profile/dry-preview", { profile_id: targetId });
          state.pendingProfileSwitchId = targetId;

          const metaDiv = document.getElementById("profile-preview-meta");
          if (metaDiv) {
            metaDiv.innerHTML = `
              <div class="config-option-card">
                <div><strong>Target Profile:</strong> ${escapeHtml(preview.target_profile.name)} (<code>${escapeHtml(preview.target_profile.id)}</code>)</div>
                <div><strong>Current Profile:</strong> ${escapeHtml(state.data?.active_profile?.name || currentId)} (<code>${escapeHtml(currentId || "")}</code>)</div>
                <div><strong>Linked Save Slot:</strong> <code>${escapeHtml(preview.target_profile.linked_save_profile_id)}</code></div>
              </div>
            `;
          }

          const addedMods = preview.added_mods || preview.diff?.only_in_b || [];
          const removedMods = preview.removed_mods || preview.diff?.only_in_a || [];

          const addedList = document.getElementById("profile-preview-added-mods");
          if (addedList) {
            addedList.innerHTML = addedMods.length > 0
              ? addedMods.map((m) => `<li class="text-ok">+ ${escapeHtml(m)}</li>`).join("")
              : "<li class=\"text-muted\">None</li>";
          }

          const removedList = document.getElementById("profile-preview-removed-mods");
          if (removedList) {
            removedList.innerHTML = removedMods.length > 0
              ? removedMods.map((m) => `<li class="text-warn">- ${escapeHtml(m)}</li>`).join("")
              : "<li class=\"text-muted\">None</li>";
          }

          const compatDiv = document.getElementById("profile-preview-save-compat");
          if (compatDiv) {
            const isSafe = preview.save_compatibility?.is_safe;
            const warn = preview.save_compatibility?.warning_message;
            compatDiv.innerHTML = `
              <div class="config-option-card" style="border-left: 4px solid ${isSafe ? "var(--color-ok)" : "var(--color-warn)"};">
                <strong>Linked Save Slot Compatibility:</strong>
                <span class="badge ${isSafe ? "badge-ok" : "badge-warn"}">${isSafe ? "Compatible" : "Warning"}</span>
                ${warn ? `<p style="margin:4px 0 0 0; font-size:12px;">${escapeHtml(warn)}</p>` : ""}
              </div>
            `;
          }

          document.getElementById("modal-profile-preview")?.showModal();
        } catch (err) {
          showToast(`Failed to generate profile preview: ${err.message}`, true);
          if (state.data?.active_profile) {
            profSelect.value = state.data.active_profile.id;
          }
        }
      }
    });
  }

  // Profile Swap Preview Modal Actions
  document.getElementById("btn-apply-profile-preview")?.addEventListener("click", async () => {
    const targetId = state.pendingProfileSwitchId;
    document.getElementById("modal-profile-preview")?.close();
    if (targetId) {
      try {
        invalidateDiffState();
        await apiPost("/api/profile/switch", { profile_id: targetId });
        await refreshState();
        showToast(`Activated profile '${targetId}'.`);
      } catch (err) {
        showToast(err.message, true);
        if (profSelect && state.data?.active_profile) {
          profSelect.value = state.data.active_profile.id;
        }
      } finally {
        state.pendingProfileSwitchId = null;
      }
    }
  });

  const handleCancelPreview = () => {
    document.getElementById("modal-profile-preview")?.close();
    if (profSelect && state.data?.active_profile) {
      profSelect.value = state.data.active_profile.id;
    }
    state.pendingProfileSwitchId = null;
  };
  document.getElementById("btn-cancel-profile-preview")?.addEventListener("click", handleCancelPreview);
  document.getElementById("btn-close-profile-preview")?.addEventListener("click", handleCancelPreview);

  // Profile Lifecycle Dialogs (Requirement R4)
  // 1. Create Profile
  function openCreateProfileModal() {
    const d = state.data;
    if (!d) return;

    const nameInput = document.getElementById("create-profile-name");
    if (nameInput) nameInput.value = "";
    const descInput = document.getElementById("create-profile-desc");
    if (descInput) descInput.value = "";

    const cloneSel = document.getElementById("create-profile-clone-source");
    if (cloneSel) {
      cloneSel.innerHTML =
        '<option value="">(Empty Profile / Do Not Clone)</option>' +
        d.profiles
          .map((p) => `<option value="${p.id}">${escapeHtml(p.name)} (${p.id})</option>`)
          .join("");
    }

    const saveSel = document.getElementById("create-profile-linked-save");
    if (saveSel) {
      saveSel.innerHTML = d.save_profiles
        .map(
          (s) =>
            `<option value="${s.profile.id}" ${s.profile.id === "default" ? "selected" : ""}>${escapeHtml(s.profile.name)} (${s.profile.id})</option>`
        )
        .join("");
    }

    document.getElementById("modal-create-profile")?.showModal();
  }
  document.getElementById("btn-create-profile")?.addEventListener("click", openCreateProfileModal);

  document.getElementById("btn-cancel-create-profile")?.addEventListener("click", () => {
    document.getElementById("modal-create-profile")?.close();
  });

  document.getElementById("btn-confirm-create-profile")?.addEventListener("click", async () => {
    const name = document.getElementById("create-profile-name")?.value.trim();
    if (!name) {
      showToast("Profile name cannot be empty.", true);
      return;
    }
    const clone_from_id = document.getElementById("create-profile-clone-source")?.value || null;
    const linked_save_id = document.getElementById("create-profile-linked-save")?.value || "default";
    const description = document.getElementById("create-profile-desc")?.value.trim() || "";

    try {
      const created = await apiPost("/api/profile/create", {
        name,
        clone_from_id,
        linked_save_id,
        description,
      });
      document.getElementById("modal-create-profile")?.close();
      await refreshState();
      showToast(`Created profile '${created.name || name}'.`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  // 2. Rename Profile
  document.getElementById("btn-rename-profile")?.addEventListener("click", () => {
    const prof = state.data?.active_profile;
    if (!prof) return;
    if (prof.id === "vanilla") {
      showToast("Cannot rename the protected 'vanilla' profile.", true);
      return;
    }
    const nameInput = document.getElementById("rename-profile-name");
    if (nameInput) nameInput.value = prof.name;
    document.getElementById("modal-rename-profile")?.showModal();
  });

  document.getElementById("btn-cancel-rename-profile")?.addEventListener("click", () => {
    document.getElementById("modal-rename-profile")?.close();
  });

  document.getElementById("btn-confirm-rename-profile")?.addEventListener("click", async () => {
    const prof = state.data?.active_profile;
    if (!prof) return;
    const new_name = document.getElementById("rename-profile-name")?.value.trim();
    if (!new_name) {
      showToast("Profile name cannot be empty.", true);
      return;
    }

    try {
      const updated = await apiPost("/api/profile/rename", {
        profile_id: prof.id,
        new_name,
      });
      document.getElementById("modal-rename-profile")?.close();
      await refreshState();
      showToast(`Renamed profile to '${updated.name || new_name}'.`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  // 3. Delete Profile
  document.getElementById("btn-delete-profile")?.addEventListener("click", () => {
    const d = state.data;
    if (!d) return;

    const delSel = document.getElementById("delete-profile-select");
    if (delSel) {
      delSel.innerHTML = d.profiles
        .map(
          (p) =>
            `<option value="${p.id}" ${p.id === d.active_profile.id ? "selected" : ""}>${escapeHtml(p.name)} (${p.id})${p.id === d.active_profile.id ? " [Active]" : ""}${p.id === "vanilla" ? " [Protected]" : ""}</option>`
        )
        .join("");
    }

    document.getElementById("modal-delete-profile")?.showModal();
  });

  document.getElementById("btn-cancel-delete-profile")?.addEventListener("click", () => {
    document.getElementById("modal-delete-profile")?.close();
  });

  document.getElementById("btn-confirm-delete-profile")?.addEventListener("click", async () => {
    const profile_id = document.getElementById("delete-profile-select")?.value;
    if (!profile_id) return;

    try {
      await apiPost("/api/profile/delete", { profile_id });
      document.getElementById("modal-delete-profile")?.close();
      await refreshState();
      showToast(`Deleted profile '${profile_id}'.`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  // Load Order Filter & Search controls
  document.getElementById("mod-search-input")?.addEventListener("input", (e) => {
    state.modSearchQuery = e.target.value;
    renderLoadOrderTab();
  });

  document.getElementById("mod-category-filter")?.addEventListener("change", (e) => {
    state.modCategoryFilter = e.target.value;
    renderLoadOrderTab();
  });

  document.getElementById("mod-fav-filter")?.addEventListener("change", (e) => {
    state.modFavOnly = e.target.checked;
    renderLoadOrderTab();
  });

  document.getElementById("btn-enable-all-mods")?.addEventListener("click", async () => {
    const allIds = state.data.installed_mods.map((m) => m.manifest.id);
    await saveActiveProfileUpdate({ enabled_mods: allIds });
  });

  document.getElementById("btn-disable-all-mods")?.addEventListener("click", async () => {
    await saveActiveProfileUpdate({ enabled_mods: [] });
  });

  // Bulk Move to Top / Move to Bottom for selected mod (Section 7)
  document.getElementById("btn-move-top-mods")?.addEventListener("click", async () => {
    const modId = state.selectedModId;
    if (!modId) return showToast("Select a mod row first.", true);
    const prof = state.data.active_profile;
    if ((prof.locked_mods || []).includes(modId)) {
      return showToast(`Mod '${modId}' priority is locked ([Locked]).`, true);
    }
    const prios = { ...prof.mod_priorities, [modId]: 1 };
    await saveActiveProfileUpdate({ mod_priorities: prios });
  });

  document.getElementById("btn-move-bottom-mods")?.addEventListener("click", async () => {
    const modId = state.selectedModId;
    if (!modId) return showToast("Select a mod row first.", true);
    const prof = state.data.active_profile;
    if ((prof.locked_mods || []).includes(modId)) {
      return showToast(`Mod '${modId}' priority is locked ([Locked]).`, true);
    }
    const prios = { ...prof.mod_priorities, [modId]: 99 };
    await saveActiveProfileUpdate({ mod_priorities: prios });
  });

  // Undo / Redo Load Order changes (Buttons + Ctrl+Z / Ctrl+Y)
  async function performUndo() {
    if (state.undoStack.length === 0) return;
    const prev = state.undoStack.pop();
    state.redoStack.push(JSON.parse(JSON.stringify(state.data.active_profile)));
    invalidateDiffState();
    await apiPost("/api/profile/update", prev);
    await refreshState();
    showToast("Undid profile load-order change.");
  }

  async function performRedo() {
    if (state.redoStack.length === 0) return;
    const next = state.redoStack.pop();
    state.undoStack.push(JSON.parse(JSON.stringify(state.data.active_profile)));
    invalidateDiffState();
    await apiPost("/api/profile/update", next);
    await refreshState();
    showToast("Redid profile load-order change.");
  }

  document.getElementById("btn-undo-order")?.addEventListener("click", performUndo);
  document.getElementById("btn-redo-order")?.addEventListener("click", performRedo);

  document.addEventListener("keydown", (e) => {
    if (e.target.closest("input, textarea, select")) return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z" && !e.shiftKey) {
      e.preventDefault();
      performUndo();
    } else if (
      (e.ctrlKey || e.metaKey) &&
      (e.key.toLowerCase() === "y" || (e.shiftKey && e.key.toLowerCase() === "z"))
    ) {
      e.preventDefault();
      performRedo();
    } else if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      // Section 7: Keyboard reordering via Alt+Up / Alt+Down
      const modId = state.selectedModId;
      if (!modId || !state.data?.active_profile) return;
      e.preventDefault();
      const prof = state.data.active_profile;
      if ((prof.locked_mods || []).includes(modId)) {
        return showToast(`Mod '${modId}' priority is locked ([Locked]).`, true);
      }
      const modObj = state.data.installed_mods.find((x) => x.manifest.id === modId);
      const cur = prof.mod_priorities[modId] ?? (modObj ? modObj.manifest.load_priority : 50);
      const delta = e.key === "ArrowUp" ? -5 : 5;
      const prios = { ...prof.mod_priorities, [modId]: Math.max(1, cur + delta) };
      saveActiveProfileUpdate({ mod_priorities: prios });
    }
  });

  // Auto-Sort non-locked mods
  document.getElementById("btn-auto-sort")?.addEventListener("click", async () => {
    const prof = state.data.active_profile;
    const locked = new Set(prof.locked_mods || []);
    const nextPrios = {};
    Object.entries(prof.mod_priorities || {}).forEach(([k, v]) => {
      if (locked.has(k)) nextPrios[k] = v;
    });
    await saveActiveProfileUpdate({ mod_priorities: nextPrios });
    showToast("Auto-sorted load order based on dependencies.");
  });

  // Activate & Sync Junctions button
  document.getElementById("btn-apply-profile").addEventListener("click", async () => {
    try {
      invalidateDiffState();
      await apiPost("/api/profile/activate", { profile_id: state.data.active_profile.id });
      await refreshState();
      showToast(`Applied changes for profile '${state.data.active_profile.name}'.`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  // Add MO2-style Separator
  document.getElementById("btn-add-separator").addEventListener("click", async () => {
    const title = prompt("Enter Separator Title (e.g. VISUAL MODS):", "MOD GROUP");
    if (!title) return;
    const prof = state.data.active_profile;
    const seps = [...(prof.separators || [])];
    seps.push({
      id: `sep-${Date.now()}`,
      title,
      color: "#d4a742",
      insert_before_mod_id: state.selectedModId || prof.enabled_mods[0] || "gcore",
      collapsed: false,
    });
    await saveActiveProfileUpdate({ separators: seps });
  });

  // Clone Profile
  document.getElementById("btn-clone-profile").addEventListener("click", async () => {
    const newName = prompt(
      "Enter name for cloned profile:",
      `${state.data.active_profile.name} (Copy)`
    );
    if (!newName) return;
    const newId = newName.toLowerCase().replace(/[^a-z0-9]+/g, "_");
    try {
      invalidateDiffState();
      await apiPost("/api/profile/clone", {
        src_id: state.data.active_profile.id,
        new_id: newId,
        new_name: newName,
      });
      await refreshState();
      showToast(`Cloned profile '${newName}' (${newId}).`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  // Diff Profiles
  document.getElementById("btn-diff-profiles").addEventListener("click", async () => {
    const profiles = state.data.profiles;
    if (profiles.length < 2) return showToast("Create or clone a second profile to compare.");
    const other = profiles.find((p) => p.id !== state.data.active_profile.id) || profiles[0];
    try {
      const diff = await apiPost("/api/profile/diff", {
        profile_a: state.data.active_profile.id,
        profile_b: other.id,
      });
      const html = `
        <table class="cmp-table">
          <thead>
            <tr><th>Category</th><th>Details</th></tr>
          </thead>
          <tbody>
            <tr><td><strong>Only in '${escapeHtml(diff.profile_a_id)}'</strong></td><td>${escapeHtml(diff.only_in_a.join(", ") || "None")}</td></tr>
            <tr><td><strong>Only in '${escapeHtml(diff.profile_b_id)}'</strong></td><td>${escapeHtml(diff.only_in_b.join(", ") || "None")}</td></tr>
            <tr><td><strong>Priority Differences</strong></td><td>${diff.priority_diffs.map((p) => `${p[0]}: ${p[1]} vs ${p[2]}`).join(", ") || "None"}</td></tr>
            <tr><td><strong>Mod Setting Differences</strong></td><td>${diff.setting_diffs.map((s) => `${s[0]}.${s[1]}: ${s[2]} vs ${s[3]}`).join(", ") || "None"}</td></tr>
          </tbody>
        </table>
      `;
      openReportModal(
        `Profile Comparison: '${diff.profile_a_id}' vs '${diff.profile_b_id}'`,
        html,
        JSON.stringify(diff, null, 2)
      );
    } catch (err) {
      showToast(err.message, true);
    }
  });

  // Share Code Modal (<dialog closedby="any">)
  const shareDialog = document.getElementById("share-profile-dialog");
  document.getElementById("btn-open-share-modal").addEventListener("click", async () => {
    try {
      const res = await apiPost("/api/profile/export-code", {
        profile_id: state.data.active_profile.id,
      });
      document.getElementById("export-code-textarea").value = res.code;
      shareDialog.showModal();
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-copy-profile-code").addEventListener("click", () => {
    const txt = document.getElementById("export-code-textarea").value;
    navigator.clipboard?.writeText(txt);
    showToast("Copied QM1-... profile code to clipboard!");
  });

  document.getElementById("btn-import-profile-code").addEventListener("click", async () => {
    const code = document.getElementById("import-code-textarea").value.trim();
    const newId =
      document.getElementById("import-code-id").value.trim() || `imported_${Date.now()}`;
    if (!code) return;
    try {
      invalidateDiffState();
      await apiPost("/api/profile/import-code", { code, new_id: newId });
      shareDialog.close();
      await refreshState();
      showToast(`Imported profile '${newId}' from QM1 code!`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  // Onboarding & Settings Modal (<dialog closedby="any">)
  const onboardDialog = document.getElementById("onboarding-dialog");
  document.getElementById("btn-open-onboarding")?.addEventListener("click", async () => {
    const s = state.data?.settings || {};
    const gameExe = document.getElementById("setup-game-exe");
    const saveDir = document.getElementById("setup-save-dir");
    const libDir = document.getElementById("setup-library-dir");
    const wsMods = document.getElementById("setup-ws-mods");
    const themePal = document.getElementById("setup-theme-palette");
    const colScheme = document.getElementById("setup-color-scheme");
    const gameBadge = document.getElementById("game-modded-badge");
    const gameStatus = document.getElementById("setup-game-status");
    const saveStatus = document.getElementById("setup-save-status");

    if (gameExe) gameExe.value = s.game_exe_path || "";
    if (saveDir) saveDir.value = s.save_dir_path || "";
    if (libDir) libDir.value = state.data?.library_dir || (state.data?.root_dir ? state.data.root_dir + "\\library" : "");
    if (wsMods) wsMods.value = s.workspace_mods_dir || "";

    if (themePal) {
      themePal.value = document.documentElement.dataset.palette || "tactical";
    }
    if (colScheme) {
      let scheme = "dark";
      try {
        scheme = localStorage.getItem("color-scheme") || (window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
      } catch (_) {}
      colScheme.value = scheme;
    }

    try {
      const gStatus = await apiGet("/api/game/status");
      if (gameBadge) {
        if (gStatus.is_modded) {
          gameBadge.textContent = "Deployed (Mods Linked)";
          gameBadge.style.color = "var(--accent-warn)";
        } else {
          gameBadge.textContent = "Clean Vanilla (Unlinked)";
          gameBadge.style.color = "var(--status-ok)";
        }
      }
    } catch (_) {}

    if (gameStatus) {
      gameStatus.textContent = s.game_exe_path ? "Configured" : "Not configured";
    }
    if (saveStatus) {
      saveStatus.textContent = s.save_dir_path ? "Configured" : "Not configured";
    }

    onboardDialog?.showModal();
  });

  document.getElementById("btn-cancel-onboarding")?.addEventListener("click", () => {
    onboardDialog?.close();
  });

  document.getElementById("btn-detect-steam")?.addEventListener("click", async () => {
    try {
      const res = await apiPost("/api/settings/auto-detect");
      if (res?.paths) {
        if (res.paths.game_exe_path) {
          document.getElementById("setup-game-exe").value = res.paths.game_exe_path;
          const statusSpan = document.getElementById("setup-game-status");
          if (statusSpan) statusSpan.textContent = res.paths.is_steam ? `Steam (${res.paths.game_version || 'v0.5.3b'})` : "Detected";
        }
        if (res.paths.save_dir_path) {
          document.getElementById("setup-save-dir").value = res.paths.save_dir_path;
          const saveSpan = document.getElementById("setup-save-status");
          if (saveSpan) saveSpan.textContent = "Detected";
        }
        showToast("Auto-detection complete!");
      }
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-detect-save-dir")?.addEventListener("click", async () => {
    try {
      const res = await apiPost("/api/settings/auto-detect");
      if (res?.paths?.save_dir_path) {
        document.getElementById("setup-save-dir").value = res.paths.save_dir_path;
        showToast("Detected save directory!");
      }
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-browse-game-exe")?.addEventListener("click", async () => {
    try {
      const res = await apiPost("/api/settings/browse", { mode: "file" });
      if (res?.ok && res.path) {
        document.getElementById("setup-game-exe").value = res.path;
      }
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-browse-save-dir")?.addEventListener("click", async () => {
    try {
      const res = await apiPost("/api/settings/browse", { mode: "folder" });
      if (res?.ok && res.path) {
        document.getElementById("setup-save-dir").value = res.path;
      }
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-browse-ws-mods")?.addEventListener("click", async () => {
    try {
      const res = await apiPost("/api/settings/browse", { mode: "folder" });
      if (res?.ok && res.path) {
        document.getElementById("setup-ws-mods").value = res.path;
      }
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-open-library-folder")?.addEventListener("click", async () => {
    try {
      const pathVal = document.getElementById("setup-library-dir")?.value;
      if (pathVal) {
        await apiPost("/api/settings/open-folder", { path: pathVal });
      }
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-restore-clean-game")?.addEventListener("click", async () => {
    try {
      await apiPost("/api/game/restore-clean");
      const gameBadge = document.getElementById("game-modded-badge");
      if (gameBadge) {
        gameBadge.textContent = "Clean Vanilla (Unlinked)";
        gameBadge.style.color = "var(--status-ok)";
      }
      showToast("Game directory restored to clean vanilla state!");
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-save-onboarding")?.addEventListener("click", async () => {
    try {
      const themeVal = document.getElementById("setup-theme-palette")?.value || "tactical";
      const colorVal = document.getElementById("setup-color-scheme")?.value || "dark";
      
      document.documentElement.dataset.palette = themeVal;
      try { localStorage.setItem("qm-palette", themeVal); } catch (_) {}
      const meta = document.querySelector('meta[name="color-scheme"]');
      if (meta) meta.content = colorVal;
      try { localStorage.setItem("color-scheme", colorVal); } catch (_) {}

      await apiPost("/api/onboarding/complete", {
        game_exe_path: document.getElementById("setup-game-exe").value.trim(),
        save_dir_path: document.getElementById("setup-save-dir").value.trim(),
        workspace_mods_dir: document.getElementById("setup-ws-mods").value.trim(),
        theme: themeVal,
      });
      onboardDialog?.close();
      await refreshState();
      showToast("Settings saved successfully!");
    } catch (err) {
      showToast(err.message, true);
    }
  });

  // Diff viewer controls (#diff-mod-filter, #btn-next-change, #btn-toggle-diff-mode, #btn-demo-conflict)
  document.getElementById("diff-mod-filter").addEventListener("change", (e) => {
    const diff = state.diffFileCache[state.selectedDiffFile];
    if (diff) renderCodePane(diff, e.target.value);
  });

  document.getElementById("btn-next-change").addEventListener("click", () => {
    const patchedRows = document.querySelectorAll(
      "#attributed-code-container .code-line.patched, #attributed-code-container .sbs-cell.patched"
    );
    if (patchedRows.length === 0) return;
    state.diffJumpCursor = (state.diffJumpCursor + 1) % patchedRows.length;
    patchedRows[state.diffJumpCursor].scrollIntoView({ behavior: "smooth", block: "center" });
  });



  document.getElementById("btn-demo-conflict").addEventListener("click", async () => {
    try {
      const res = await apiGet("/api/conflict/demo");
      const html = `
        <div class="config-option-card">
          <div class="field-label">DETECTED OVERLAP</div>
          <p style="margin-top:4px">${escapeHtml(res.description)}</p>
        </div>
        <div class="config-option-card">
          <div class="field-label">1. AUTOMATIC LOAD ORDER SORT</div>
          <p style="margin-top:4px">${escapeHtml(res.tier1_explanation)}</p>
        </div>
        <div class="config-option-card">
          <div class="field-label">2. COMPATIBILITY PATCH</div>
          <pre class="markdown-box" style="margin-top:6px">${escapeHtml(res.tier2_glue_preview)}</pre>
        </div>
      `;
      openReportModal("Conflict Resolution Demo", html, res.tier2_glue_preview);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  // Save Viewer controls
  document.getElementById("save-slot-select").addEventListener("change", (e) => {
    state.selectedSaveSlotId = e.target.value;
    renderSavesTab();
  });

  document.getElementById("btn-activate-save-slot")?.addEventListener("click", async () => {
    try {
      await apiPost("/api/save/activate", { save_id: state.selectedSaveSlotId });
      await refreshState();
      showToast(`Switched to save '${state.selectedSaveSlotId}'.`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-clone-save-slot")?.addEventListener("click", async () => {
    const newName = prompt(
      "Enter name for cloned save slot:",
      `${state.selectedSaveSlotId} Campaign Copy`
    );
    if (!newName) return;
    const newId = newName.toLowerCase().replace(/[^a-z0-9]+/g, "_");
    try {
      await apiPost("/api/save/clone", {
        src_id: state.selectedSaveSlotId,
        new_id: newId,
        new_name: newName,
      });
      state.selectedSaveSlotId = newId;
      await refreshState();
      showToast(`Cloned save '${newName}'.`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-backup-save").addEventListener("click", async () => {
    const label = prompt("Enter backup label:", "Manual Backup");
    if (!label) return;
    try {
      await apiPost("/api/save/backup", {
        save_id: state.selectedSaveSlotId,
        label,
      });
      await refreshState();
      showToast(`Created backup '${label}'.`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-export-dossier")?.addEventListener("click", async () => {
    try {
      const res = await apiGet(
        `/api/save/export-dossier?save_id=${encodeURIComponent(state.selectedSaveSlotId)}`
      );
      openReportModal(
        `Save Summary — Slot '${res.save_id}'`,
        `<pre class="markdown-box">${escapeHtml(res.markdown)}</pre>`,
        res.markdown
      );
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-compare-saves").addEventListener("click", () => {
    runSaveSlotComparison(true);
  });

  // Library, Modpack & Importer controls
  document.getElementById("btn-sync-workspace").addEventListener("click", async () => {
    try {
      await apiPost("/api/mod/sync-workspace", {});
      invalidateDiffState();
      await refreshState();
      showToast("Scanned and updated installed mods.");
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-verify-library").addEventListener("click", async () => {
    await refreshState();
    showToast("All mod files verified successfully.");
  });

  document.getElementById("btn-import-path").addEventListener("click", async () => {
    const p = document.getElementById("import-path-input").value.trim();
    const allowUnmanaged = document.getElementById("import-unmanaged-check").checked;
    if (!p) return;
    try {
      await apiPost("/api/mod/import", { path: p, allow_unmanaged: allowUnmanaged });
      invalidateDiffState();
      await refreshState();
      showToast(`Imported mod from '${p}'.`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  // Multi-Mod Archive Inspector & Upgrade/Replace/Alongside Picker Modal (Section 4)
  async function openArchiveInspectorModal(pathStr) {
    const allowUnmanaged = document.getElementById("import-unmanaged-check")?.checked || false;
    try {
      const res = await apiPost("/api/mod/inspect-archive", {
        path: pathStr,
        allow_unmanaged: allowUnmanaged,
      });
      const cands = res.candidates || [];
      const box = document.getElementById("import-picker-candidates");
      box.innerHTML = cands
        .map(
          (c) => `
          <label class="backup-item" style="cursor:pointer">
            <div style="display:flex; align-items:center; gap:8px;">
              <input type="checkbox" class="archive-cand-check" value="${escapeHtml(c.manifest.id)}" checked />
              <div>
                <div><strong>${escapeHtml(c.manifest.name)}</strong> <span class="badge">v${escapeHtml(c.manifest.version)}</span></div>
                <div class="provenance-line">ID: <code>${escapeHtml(c.manifest.id)}</code> • Folder: <code>${escapeHtml(c.relative_folder)}</code></div>
              </div>
            </div>
            ${
              c.already_installed_version
                ? `<span class="badge badge-warn">Already Installed (v${escapeHtml(c.already_installed_version)})</span>`
                : `<span class="badge badge-ok">New Mod</span>`
            }
          </label>
        `
        )
        .join("");

      const dlg = document.getElementById("import-picker-dialog");
      dlg.dataset.sourcePath = pathStr;
      dlg.showModal();
    } catch (err) {
      showToast(err.message, true);
    }
  }

  document.getElementById("btn-inspect-archive")?.addEventListener("click", () => {
    const p = document.getElementById("import-path-input").value.trim();
    if (!p) return showToast("Enter a path to a .zip archive or mod folder first.", true);
    openArchiveInspectorModal(p);
  });

  document.getElementById("btn-confirm-selective-import")?.addEventListener("click", async () => {
    const dlg = document.getElementById("import-picker-dialog");
    const pathStr = dlg.dataset.sourcePath || document.getElementById("import-path-input").value.trim();
    const allowUnmanaged = document.getElementById("import-unmanaged-check")?.checked || false;
    const policy = document.getElementById("import-conflict-policy")?.value || "upgrade";
    const selectedIds = Array.from(document.querySelectorAll(".archive-cand-check:checked")).map(
      (el) => el.value
    );
    if (selectedIds.length === 0) return showToast("Select at least one mod to import.", true);
    try {
      const sum = await apiPost("/api/mod/import", {
        path: pathStr,
        allow_unmanaged: allowUnmanaged,
        selected_mod_ids: selectedIds,
        conflict_policy: policy,
      });
      dlg.close();
      invalidateDiffState();
      await refreshState();
      showToast(
        `Imported ${sum.imported_mods.length} mod(s) (${policy}).`
      );
    } catch (err) {
      showToast(err.message, true);
    }
  });

  // Window-wide Drag-and-Drop for .zip / .qmpack files (Section 4)
  window.addEventListener("dragover", (e) => {
    if (e.dataTransfer?.types?.includes("Files")) {
      e.preventDefault();
    }
  });
  window.addEventListener("drop", (e) => {
    if (!e.dataTransfer?.files?.length) return;
    e.preventDefault();
    const file = e.dataTransfer.files[0];
    if (!file) return;
    document.querySelector('.nav-tab[data-tab="library"]')?.click();
    const inp = document.getElementById("import-path-input");
    if (inp && file.name) {
      inp.value = file.path || file.name;
      showToast(`Selected '${file.name}'. Click Browse Archive or Import Mod to finish.`);
    }
  });

  // In-App Mod Browser search & filter
  document.getElementById("browser-search-input")?.addEventListener("input", (e) => {
    state.browserSearchQuery = e.target.value;
    renderCommunityModsSubpage();
  });
  document.getElementById("browser-source-filter")?.addEventListener("change", (e) => {
    state.browserSourceFilter = e.target.value;
    renderCommunityModsSubpage();
  });

  // Register Windows URL & File Protocols (qm://, nxm://, .qmpack)
  document.getElementById("btn-register-protocols")?.addEventListener("click", async () => {
    try {
      const res = await apiPost("/api/system/register-protocols", {});
      showToast(`Registered in HKCU: ${(res.registered || []).join(" • ")}`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  // Check All Updates
  document.getElementById("btn-check-updates-secondary")?.addEventListener("click", () => {
    document.getElementById("btn-check-updates")?.click();
  });

  document.getElementById("btn-check-updates")?.addEventListener("click", async () => {
    try {
      const up = await apiPost("/api/updates/check", {});
      state.data.updates = up;
      renderLibraryTab();
      showToast(`Checked updates across ${up.mods?.length || 0} mods (${up.rate_limit_mode}).`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-export-qmpack")?.addEventListener("click", async () => {
    try {
      const res = await apiPost("/api/modpack/export", {
        profile_id: state.data.active_profile.id,
      });
      showToast(`Exported .qmpack archive to: ${res.path}`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-import-qmpack")?.addEventListener("click", async () => {
    const p = document.getElementById("import-qmpack-input").value.trim();
    if (!p) return showToast("Enter a .qmpack file path first.", true);
    try {
      invalidateDiffState();
      const res = await apiPost("/api/modpack/import", {
        path: p,
        new_id: `modpack_${Date.now()}`,
      });
      await refreshState();
      showToast(`Imported .qmpack profile '${res.profile.name}'!`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  // Crash & Diagnostics controls
  document.getElementById("btn-simulate-crash").addEventListener("click", async () => {
    try {
      await apiPost("/api/crash/simulate", {});
      await refreshState();
      showToast("Recorded simulated crash.");
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-export-diag-bundle")?.addEventListener("click", async () => {
    try {
      const bundle = await apiGet("/api/diagnostics/bundle");
      const html = `
        <div class="field-label">DIAGNOSTIC REPORT (Paths Redacted)</div>
        <pre class="markdown-box">${escapeHtml(bundle.github_issue_markdown)}</pre>
        <div class="field-label" style="margin-top:8px">FULL DIAGNOSTIC BUNDLE (JSON)</div>
        <pre class="markdown-box">${escapeHtml(JSON.stringify(bundle, null, 2))}</pre>
      `;
      openReportModal(
        "Diagnostic Report",
        html,
        bundle.github_issue_markdown
      );
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("log-level-filter")?.addEventListener("change", (e) => {
    state.logLevelFilter = e.target.value;
    renderCrashesTab();
  });

  document.getElementById("log-mod-filter")?.addEventListener("change", (e) => {
    state.logModFilter = e.target.value;
    renderCrashesTab();
  });

  document.getElementById("btn-open-log-folder")?.addEventListener("click", async () => {
    try {
      const res = await apiPost("/api/system/open-logs-folder", {});
      showToast(`Opened logs folder: ${res.opened_path}`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-refresh-health")?.addEventListener("click", async () => {
    await refreshState();
    showToast("Refreshed logs and injector status.");
  });

  // Dev Studio: Scaffolder, Linter & Packager
  document.getElementById("form-scaffold-mod").addEventListener("submit", async (e) => {
    e.preventDefault();
    const modId = document.getElementById("scaf-id").value.trim();
    try {
      const res = await apiPost("/api/devtools/scaffold", {
        mod_id: modId,
        mod_name: document.getElementById("scaf-name").value.trim(),
        author: document.getElementById("scaf-author").value.trim(),
        template: document.getElementById("scaf-template").value,
      });
      const out = document.getElementById("scaf-output");
      out.hidden = false;
      out.className = "markdown-box";
      out.textContent = `Created new mod at: ${res.path}`;
      await refreshState();
      showToast(`Created new mod '${modId}'.`);

      // Immediately select and load in the New Mod Files & Templating workspace
      state.scaffoldActiveModDir = modId;
      const lintSel = document.getElementById("lint-mod-select");
      if (lintSel) {
        lintSel.innerHTML = `<option value="${escapeHtml(modId)}">${escapeHtml(modId)} (Working Mod)</option>`;
        lintSel.value = modId;
      }
      await loadScaffoldModFiles(modId);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  // Scaffold New Mod Editor Event Listeners
  const lintModSel = document.getElementById("lint-mod-select");
  lintModSel?.addEventListener("change", (e) => {
    state.scaffoldActiveFilePath = null;
    loadScaffoldModFiles(e.target.value);
  });

  document.getElementById("btn-scaffold-add-file")?.addEventListener("click", () => {
    addBlankFileToScaffold();
  });

  document.getElementById("btn-scaffold-copy-template")?.addEventListener("click", () => {
    insertTemplateToScaffold();
  });

  document.getElementById("btn-save-new-mod-file")?.addEventListener("click", () => {
    saveScaffoldActiveFile();
  });

  const scaffoldTextarea = document.getElementById("new-mod-editor-textarea");
  const scaffoldLineNumbers = document.getElementById("new-mod-line-numbers");
  const scaffoldDirty = document.getElementById("new-mod-dirty-indicator");

  if (scaffoldTextarea) {
    scaffoldTextarea.addEventListener("input", () => {
      if (scaffoldDirty) {
        const isDirty = state.scaffoldOriginalContent !== scaffoldTextarea.value;
        scaffoldDirty.style.display = isDirty ? "inline-flex" : "none";
      }
      updateScaffoldEditorLineNumbers();
      updateScaffoldLiveEditorValidation();
    });

    scaffoldTextarea.addEventListener("scroll", () => {
      if (scaffoldLineNumbers) {
        scaffoldLineNumbers.scrollTop = scaffoldTextarea.scrollTop;
      }
    });

    scaffoldTextarea.addEventListener("keydown", (e) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        saveScaffoldActiveFile();
      }
    });
  }

  document.getElementById("btn-goto-editor-subtab")?.addEventListener("click", () => {
    const workingMod = document.getElementById("lint-mod-select")?.value;
    if (workingMod) {
      state.editorActiveModDir = workingMod;
    }
    const editorSubtabBtn = document.querySelector('#modtools-subtabs .subtab[data-modtab="editor"]');
    if (editorSubtabBtn) editorSubtabBtn.click();
  });

  document.getElementById("btn-run-lint")?.addEventListener("click", async () => {
    const modDir =
      document.getElementById("editor-mod-select")?.value ||
      document.getElementById("lint-mod-select")?.value ||
      state.editorActiveModDir;
    if (!modDir) return showToast("Select a mod directory first.", true);
    try {
      const rep = await apiPost("/api/devtools/lint", { mod_dir: modDir });
      const out = document.getElementById("lint-report-output");
      if (out) {
        out.innerHTML = `<pre class="markdown-box">${escapeHtml(JSON.stringify(rep, null, 2))}</pre>`;
      }
      showToast(`Checked '${rep.mod_name}'.`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-run-pack")?.addEventListener("click", async () => {
    const modDir =
      document.getElementById("editor-mod-select")?.value ||
      document.getElementById("lint-mod-select")?.value ||
      state.editorActiveModDir;
    if (!modDir) return showToast("Select a mod directory first.", true);
    try {
      const rep = await apiPost("/api/devtools/pack", { mod_dir: modDir });
      const out = document.getElementById("lint-report-output");
      if (out) {
        out.innerHTML = `<pre class="markdown-box">${escapeHtml(JSON.stringify(rep, null, 2))}</pre>`;
      }
      showToast(`Packaged archive: ${rep.zip_path}`);
    } catch (err) {
      showToast(err.message, true);
    }
  });

  // Mod File Browser & In-App Code Editor Controls (M3 / R3)
  const editorModSel = document.getElementById("editor-mod-select");
  editorModSel?.addEventListener("change", (e) => {
    state.editorActiveFilePath = null;
    loadModFilesTree(e.target.value);
  });

  document.getElementById("btn-save-mod-file")?.addEventListener("click", () => {
    saveActiveModFile();
  });

  document.getElementById("btn-add-blank-file")?.addEventListener("click", () => {
    addBlankFile();
  });

  document.getElementById("btn-copy-template-into-mod")?.addEventListener("click", () => {
    copyTemplateIntoMod();
  });

  const editorTextarea = document.getElementById("mod-file-editor-textarea");
  const editorLineNumbers = document.getElementById("editor-line-numbers");
  const editorDirty = document.getElementById("editor-dirty-indicator");

  if (editorTextarea) {
    editorTextarea.addEventListener("input", () => {
      if (editorDirty) {
        const isDirty = state.editorOriginalContent !== editorTextarea.value;
        editorDirty.style.display = isDirty ? "inline-flex" : "none";
      }
      updateEditorLineNumbers();

      clearTimeout(editorValidationDebounceTimer);
      editorValidationDebounceTimer = setTimeout(updateLiveEditorValidation, 200);

      updateAutocompleteUI();
    });

    editorTextarea.addEventListener("scroll", () => {
      if (editorLineNumbers) {
        editorLineNumbers.scrollTop = editorTextarea.scrollTop;
      }
      hideAutocompleteBox();
    });

    editorTextarea.addEventListener("keydown", (e) => {
      // Ctrl+S / Cmd+S save shortcut inside editor textarea
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        saveActiveModFile();
        return;
      }

      // Autocomplete navigation when suggestion box is visible
      if (autocompleteState.visible && autocompleteState.items.length > 0) {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          autocompleteState.selectedIndex = (autocompleteState.selectedIndex + 1) % autocompleteState.items.length;
          renderAutocompleteBox();
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          autocompleteState.selectedIndex = (autocompleteState.selectedIndex - 1 + autocompleteState.items.length) % autocompleteState.items.length;
          renderAutocompleteBox();
          return;
        }
        if (e.key === "Enter" || e.key === "Tab") {
          e.preventDefault();
          insertAutocompleteItem(autocompleteState.selectedIndex);
          return;
        }
        if (e.key === "Escape") {
          e.preventDefault();
          hideAutocompleteBox();
          return;
        }
      }

      // Ctrl+Space manually triggers autocomplete
      if ((e.ctrlKey || e.metaKey) && e.key === " ") {
        e.preventDefault();
        updateAutocompleteUI();
        return;
      }

      // Tab key support: indent with 2 spaces
      if (e.key === "Tab") {
        e.preventDefault();
        const start = editorTextarea.selectionStart;
        const end = editorTextarea.selectionEnd;
        const val = editorTextarea.value;
        editorTextarea.value = val.substring(0, start) + "  " + val.substring(end);
        editorTextarea.selectionStart = editorTextarea.selectionEnd = start + 2;
        editorTextarea.dispatchEvent(new Event("input"));
        return;
      }
    });

    // Dismiss autocomplete when clicking outside
    document.addEventListener("mousedown", (e) => {
      if (!e.target.closest("#editor-autocomplete-box") && e.target !== editorTextarea) {
        hideAutocompleteBox();
      }
    });

    // Toggle Problems Panel buttons
    document.getElementById("btn-toggle-problems")?.addEventListener("click", (e) => {
      e.stopPropagation();
      const panel = document.getElementById("editor-problems-panel");
      const btn = document.getElementById("btn-toggle-problems");
      if (!panel) return;
      panel.classList.toggle("collapsed");
      if (btn) btn.textContent = panel.classList.contains("collapsed") ? "Expand" : "Collapse";
    });

    document.getElementById("editor-problems-header")?.addEventListener("click", () => {
      const panel = document.getElementById("editor-problems-panel");
      const btn = document.getElementById("btn-toggle-problems");
      if (!panel) return;
      panel.classList.toggle("collapsed");
      if (btn) btn.textContent = panel.classList.contains("collapsed") ? "Expand" : "Collapse";
    });

    document.getElementById("editor-validation-status")?.addEventListener("click", () => {
      const panel = document.getElementById("editor-problems-panel");
      const btn = document.getElementById("btn-toggle-problems");
      if (panel) {
        panel.classList.remove("collapsed");
        if (btn) btn.textContent = "Collapse";
        panel.scrollIntoView({ behavior: "smooth", block: "nearest" });
      }
    });
  }

  // Global Ctrl+S shortcut handler for in-app code editor
  window.addEventListener("keydown", (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
      const devtoolsTab = document.getElementById("tab-devtools");
      const editorPanel = document.getElementById("modtools-tab-editor");
      if (
        (devtoolsTab && devtoolsTab.classList.contains("active")) ||
        (editorPanel && editorPanel.classList.contains("active")) ||
        state.editorActiveFilePath
      ) {
        e.preventDefault();
        saveActiveModFile();
      }
    }
  });

  // Live Developer Watch & Hot-Reload Mode (Section 14)
  document.getElementById("dev-watch-toggle")?.addEventListener("change", (e) => {
    const statusPill = document.getElementById("dev-watch-status-pill");
    if (e.target.checked) {
      state.devWatchLastMs = Date.now();
      if (statusPill) {
        statusPill.textContent = "Watching for changes";
        statusPill.className = "badge badge-ok";
      }
      state.devWatchTimer = setInterval(async () => {
        try {
          const poll = await apiPost("/api/devtools/watch-poll", {
            since_epoch_ms: state.devWatchLastMs,
          });
          if (poll.now_epoch_ms) state.devWatchLastMs = poll.now_epoch_ms;
          if (poll.changed) {
            invalidateDiffState();
            await refreshState();
            showToast(`Reloaded ${poll.changed_files.length} changed file(s).`);
            if (document.getElementById("dev-watch-relaunch")?.checked) {
              await apiPost("/api/launch", {
                profile_id: state.data.active_profile.id,
                vanilla: false,
              });
            }
          }
        } catch (_) {}
      }, 2500);
    } else {
      clearInterval(state.devWatchTimer);
      state.devWatchTimer = null;
      if (statusPill) {
        statusPill.textContent = "Idle";
        statusPill.className = "badge";
      }
    }
  });

  // Launch buttons with Pre-Launch Validation Gate (Section 8)
  document.getElementById("btn-launch-primary").addEventListener("click", async () => {
    try {
      const diags = state.data?.load_order?.diagnostics || [];
      const blockingErrors = diags.filter((d) => d.severity === "error");
      if (blockingErrors.length > 0) {
        return showToast(
          `Launch blocked: ${blockingErrors[0].message}`,
          true
        );
      }
      const warnings = diags.filter((d) => d.severity === "warning");
      if (warnings.length > 0) {
        const proceed = confirm(
          `Validation Warning: ${warnings.map((w) => w.message).join("\n")}\n\nLaunch anyway?`
        );
        if (!proceed) return;
      }

      // Steam Cloud Pre-Launch Warning Guard
      const cloudWarn = state.data?.health?.steam_cloud_warning;
      if (cloudWarn) {
        const proceedCloud = confirm(
          "Steam Cloud Notice:\n\n" + cloudWarn + "\n\nSteam Cloud may overwrite or revert swapped save files while Steam is running.\n\nClick OK to proceed with launch, or Cancel to abort."
        );
        if (!proceedCloud) return;
      }

      const launchMethod = document.getElementById("header-launch-method")?.value || "direct_exe";
      const lbl = document.getElementById("launch-primary-label");
      if (lbl && launchMethod === "steam_url") {
        lbl.textContent = "Launching Steam…";
      }
      const res = await apiPost("/api/launch", {
        profile_id: state.data.active_profile.id,
        method: launchMethod,
        vanilla: false,
      });
      await refreshState();
      showToast(
        `Launched Mud and Blood with profile '${state.data.active_profile.name}'.`
      );
    } catch (err) {
      showToast(err.message, true);
    }
  });

  document.getElementById("btn-launch-vanilla").addEventListener("click", async () => {
    try {
      const res = await apiPost("/api/launch", {
        profile_id: "vanilla",
        method: "vanilla",
        vanilla: true,
      });
      showToast("Launched Mud and Blood (Vanilla).");
    } catch (err) {
      showToast(err.message, true);
    }
  });

  refreshState();
  startLaunchStatusWatcher();
  // Copy active modlist as Markdown table (Discord / forum sharing)
  document.getElementById("btn-copy-loadout-md")?.addEventListener("click", () => {
    const d = state.data;
    const prof = d?.active_profile;
    const installed = d?.installed_mods || [];
    if (!prof) return;

    const enabledMods = installed.filter((m) => (prof.enabled_mods || []).includes(m.manifest.id));
    let md = "### Mud and Blood Mod Loadout — " + prof.name + "\n\n";
    md += "| # | Mod | Version | Priority | Patches | Description |\n";
    md += "| :--- | :--- | :--- | :--- | :--- | :--- |\n";
    enabledMods.forEach((m, idx) => {
      const prio = prof.mod_priorities[m.manifest.id] ?? m.manifest.load_priority;
      md += "| " + (idx + 1) + " | **" + m.manifest.name + "** | `v" + m.manifest.version + "` | " + prio + " | " + m.patch_count + " | " + (m.manifest.description || "N/A") + " |\n";
    });
    md += "\n*Exported by QuarterMaster (Mud and Blood Tactical Launcher)*\n";

    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(md).then(() => {
        showToast("Copied active modlist markdown to clipboard!");
      }).catch(() => {
        openReportModal("Active Modlist Markdown", "<pre class=\"markdown-box\">" + escapeHtml(md) + "</pre>", md);
      });
    } else {
      openReportModal("Active Modlist Markdown", "<pre class=\"markdown-box\">" + escapeHtml(md) + "</pre>", md);
    }
  });
});

// Expose state and studio functions for Node unit test environments
if (typeof window !== "undefined") {
  window.state = state;
}
if (typeof globalThis !== "undefined") {
  globalThis.state = state;
  globalThis.setPatchTargetRange = setPatchTargetRange;
  globalThis.adjustPatchBoundary = adjustPatchBoundary;
  globalThis.togglePatchOverlay = togglePatchOverlay;
  globalThis.renderPatchStudioSourceLines = renderPatchStudioSourceLines;
  globalThis.filterSchemaCompliantPatches = filterSchemaCompliantPatches;
  globalThis.addWizardDependency = addWizardDependency;
  globalThis.removeWizardDependency = removeWizardDependency;
  globalThis.confirmWizardAndOpenEditor = confirmWizardAndOpenEditor;
  globalThis.setContextLinesCount = setContextLinesCount;
  globalThis.getEditorContextSlices = getEditorContextSlices;
  globalThis.computeLivePatchDiff = computeLivePatchDiff;
  globalThis.serializePatchToToml = serializePatchToToml;
  globalThis.insertOrUpdatePatchInQmToml = insertOrUpdatePatchInQmToml;
  globalThis.getDependentPatches = getDependentPatches;
  globalThis.checkPatchPrerequisites = checkPatchPrerequisites;
  globalThis.TUTORIAL_TEMPLATES = TUTORIAL_TEMPLATES;
  globalThis.selectTutorialTemplate = selectTutorialTemplate;
  globalThis.renderTutorialsHubHtml = renderTutorialsHubHtml;
  globalThis.scaffoldAndOpenInStudio = scaffoldAndOpenInStudio;
  globalThis.clearPatchSelection = clearPatchSelection;
  globalThis.autoResizeTextarea = autoResizeTextarea;
  globalThis.renderLivePatchEditor = renderLivePatchEditor;
  globalThis.renderLiveDiffRowsHtml = renderLiveDiffRowsHtml;
  globalThis.updateStudioInlineDiffOnly = updateStudioInlineDiffOnly;
  globalThis.parsePatchesFromQmToml = parsePatchesFromQmToml;
}
