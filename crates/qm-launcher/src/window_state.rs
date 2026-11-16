//! The main window's place, size and maximized state, kept in QuarterMaster's own data folder
//! (`window.json`) so nothing about the launcher is left elsewhere.
use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

const FILE: &str = "window.json";
/// The smallest size worth restoring; anything below is a damaged file.
const MIN_SIZE: (u32, u32) = (400, 300);
/// How much of the window's top edge must be on a screen for its place to be restored.
const GRAB: i32 = 80;

/// In physical pixels: the window's outer position and inner size.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Saved {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

/// A screen's place and size in physical pixels.
pub type Screen = (i32, i32, u32, u32);

pub fn load(dir: &Path) -> Option<Saved> {
    let raw = std::fs::read_to_string(dir.join(FILE)).ok()?;
    let saved: Saved = serde_json::from_str(&raw).ok()?;
    (saved.width >= MIN_SIZE.0 && saved.height >= MIN_SIZE.1).then_some(saved)
}

pub fn save(dir: &Path, saved: &Saved) {
    match serde_json::to_string(saved) {
        Ok(json) => {
            if let Err(e) = std::fs::write(dir.join(FILE), json) {
                tracing::warn!("could not save the window's place: {e}");
            }
        }
        Err(e) => tracing::warn!("could not save the window's place: {e}"),
    }
}

/// Whether a strip along the window's top edge is on one of `screens`, so the window can be
/// reached and dragged. A screen that was unplugged since must not leave it out of sight.
pub fn on_screen(saved: &Saved, screens: &[Screen]) -> bool {
    let width = i32::try_from(saved.width).unwrap_or(i32::MAX);
    screens.iter().any(|&(sx, sy, sw, sh)| {
        let right = sx.saturating_add(i32::try_from(sw).unwrap_or(i32::MAX));
        let bottom = sy.saturating_add(i32::try_from(sh).unwrap_or(i32::MAX));
        let overlap = saved.x.saturating_add(width).min(right) - saved.x.max(sx);
        overlap >= GRAB && saved.y >= sy - 8 && saved.y < bottom - GRAB
    })
}

/// Puts the window where it was. A place that is on no screen any more is not restored.
pub fn restore(window: &WebviewWindow, dir: &Path) {
    let Some(saved) = load(dir) else {
        return;
    };
    if let Err(e) = window.set_size(PhysicalSize::new(saved.width, saved.height)) {
        tracing::warn!("could not restore the window's size: {e}");
    }
    let screens: Vec<Screen> = window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            (
                m.position().x,
                m.position().y,
                m.size().width,
                m.size().height,
            )
        })
        .collect();
    if on_screen(&saved, &screens) {
        if let Err(e) = window.set_position(PhysicalPosition::new(saved.x, saved.y)) {
            tracing::warn!("could not restore the window's place: {e}");
        }
    }
    if saved.maximized {
        if let Err(e) = window.maximize() {
            tracing::warn!("could not maximize the window: {e}");
        }
    }
}

/// Notes the window's place as it closes. A maximized window keeps the place and size it had
/// before, so leaving maximized goes back to them; a minimized one changes nothing.
pub fn capture(window: &WebviewWindow, dir: &Path) {
    if window.is_minimized().unwrap_or(false) {
        return;
    }
    let maximized = window.is_maximized().unwrap_or(false);
    let now = match (window.outer_position(), window.inner_size()) {
        (Ok(p), Ok(s)) => Some(Saved {
            x: p.x,
            y: p.y,
            width: s.width,
            height: s.height,
            maximized,
        }),
        _ => None,
    };
    let saved = match (maximized, load(dir), now) {
        (true, Some(before), _) => Saved {
            maximized: true,
            ..before
        },
        (_, _, Some(now)) => now,
        _ => return,
    };
    save(dir, &saved);
}
