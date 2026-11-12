// Applies the last saved appearance (theme, colour vision, contrast, text size) before the app
// mounts, so the window never flashes the wrong theme or size. Written by applyAppearance in
// src/lib/settings/appearance.ts; settings.json stays the source of truth. A plain file because
// the CSP blocks inline scripts.
(function () {
  "use strict";
  var a;
  try {
    a = JSON.parse(localStorage.getItem("qm.appearance") || "null");
  } catch (e) {
    return;
  }
  if (!a || typeof a !== "object") return;
  var root = document.documentElement;
  var THEMES = { dark: 1, light: 1 };
  var CVD = { deutan: 1, protan: 1, tritan: 1, mono: 1 };
  if (THEMES[a.theme] === 1) root.setAttribute("data-theme", a.theme);
  if (CVD[a.cvd] === 1) root.setAttribute("data-cvd", a.cvd);
  if (a.contrast === "high") root.setAttribute("data-contrast", "high");
  var vars = a.vars && typeof a.vars === "object" ? a.vars : {};
  for (var name in vars) {
    var value = vars[name];
    // Only our own size tokens, and only plain pixel values.
    if (/^--qm-[a-z0-9-]+$/.test(name) && typeof value === "string" && /^\d+(\.\d+)?px$/.test(value)) {
      root.style.setProperty(name, value);
    }
  }
})();
