//! Ordnance (grenades, rifle grenades, mortars, artillery, air strikes, drops) and class
//! skills: the hooks that let `QM.Ordnance`, `QM.Explosions` (`ordnance_runtime.lua`) and
//! `QM.Skills` (`skills_runtime.lua`) work inside the game's code. They are added only when a
//! loaded mod lists `ordnance` or `skills` in `qm_features` (or an active faction has ordnance
//! or skills), so a game without such a mod is patched as before.

use crate::patch::menu::pattern_patch;
use crate::patch::plan::ModMeta;
use crate::patch::{InsertPosition, Patch};

/// The feature names a mod lists to get the hooks.
pub const FEATURES: [&str; 2] = ["ordnance", "skills"];

/// Where the hooks go: (id, name, target, the game's lines, payload, after them or before,
/// how many times the game's lines appear in their file).
pub const HOOKS: [(&str, &str, &str, &str, &str, bool, usize); 8] = [
    (
        "qm_ordnance_click",
        "QuarterMaster Ordnance: the click after a skill is armed",
        "main.lua",
        "if(mousePrompt == 1) then",
        "if QM and QM.Ordnance and QM.Ordnance.click then QM.Ordnance.click(x, y) end",
        false,
        1,
    ),
    (
        "qm_ordnance_throw",
        "QuarterMaster Ordnance: a soldier's throw",
        "includes/ai.lua",
        "if(player.deployedTroops[i].grenadeType == 1) then",
        "if QM and QM.Ordnance and QM.Ordnance.throwFor then QM.Ordnance.throwFor(i) end",
        false,
        1,
    ),
    (
        "qm_ordnance_blast",
        "QuarterMaster Ordnance: the blast",
        "includes/particlesSystem.lua",
        r#"if(ticle.sprite == "frag" or ticle.sprite == "fragVc") then"#,
        r#"if QM and QM.Ordnance and QM.Ordnance.blast and QM.Ordnance.blast(ticle, foox, fooy) then ticle.sprite = "" end"#,
        false,
        1,
    ),
    (
        "qm_explosions_make",
        "QuarterMaster Explosions: registered explosions",
        "includes/urb_engine.lua",
        r#"elseif(tag == "flakExplosion") then"#,
        "elseif(QM and QM.Explosions and QM.Explosions.defs[tag]) then\n  clone_sfx = QM.Explosions.make(tag, x, y, dad)",
        false,
        1,
    ),
    (
        "qm_skills_button",
        "QuarterMaster Skills: the skill buttons",
        "includes/urb_engine.lua",
        "buttons[#buttons+1] = Button.new(player.screenx+(peeps*60), 610+player.screeny, 45, 45,\"skill_1\",\"active\", true, tellTime(player.deployedTroops[convertedId(specID)].skill1Timer))\nend\nend",
        "if QM and QM.Skills and QM.Skills.addButton then QM.Skills.addButton(peeps) end",
        true,
        1,
    ),
    (
        // A rifle grenade lands (where it was aimed, or on a wall): the game's two branches.
        "qm_ordnance_rifle_blast",
        "QuarterMaster Ordnance: a rifle grenade's impact",
        "includes/particlesSystem.lua",
        r#"elseif(ticle.typez == "rifleGrenade") then"#,
        "elseif(ticle.typez == \"rifleGrenade\" and ticle.qmOrdnance and QM and QM.Ordnance and QM.Ordnance.blast and QM.Ordnance.blast(ticle, ticle.x, ticle.y)) then\n  table.remove(player.particles, i)",
        false,
        2,
    ),
    (
        // The game's mortar queued a round's createExplosion: a mod's mortar changes it.
        "qm_ordnance_mortar_round",
        "QuarterMaster Ordnance: a mortar round",
        "includes/ai.lua",
        "player.delayedAction[#player.delayedAction].arg3 = player.deployedTroops[i].fragy + (randNbr(300) - 150)",
        "if QM and QM.Ordnance and QM.Ordnance.mortarRound then QM.Ordnance.mortarRound(player.deployedTroops[i], player.delayedAction[#player.delayedAction]) end",
        true,
        1,
    ),
    (
        // The promotion's skill choice: skill levels past the game's third.
        "qm_skills_select",
        "QuarterMaster Skills: skill levels past the third",
        "includes/urb_engine.lua",
        "if(player.deployedTroops[convertedId(specID)].skillUpgrade < 3) then
buttons[#buttons+1] = Button.new(player.screenx+(peeps*60), 610+player.screeny, 45, 45,\"skillSelect\",\"active\", true, _G[skillTextSelector()])
end",
        "if QM and QM.Skills and QM.Skills.addSelect then QM.Skills.addSelect(peeps) end",
        true,
        1,
    ),
];

/// The hooks, when any loaded mod lists the feature or an active faction has ordnance or
/// explosions; nothing otherwise.
pub fn build_ordnance_patches(mods: &[ModMeta], faction_ordnance: bool) -> Vec<Patch> {
    if !faction_ordnance && !mods.iter().any(|m| FEATURES.iter().any(|f| m.features.contains(*f))) {
        return Vec::new();
    }
    HOOKS
        .iter()
        .map(|(id, name, target, lines, payload, after, times)| {
            let position = if *after { InsertPosition::After } else { InsertPosition::Before };
            let mut patch = pattern_patch(id, name, target, lines, position, payload.to_string());
            if let Patch::Pattern(p) = &mut patch {
                p.times = Some(*times);
            }
            patch
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(features: &[&str]) -> ModMeta {
        ModMeta {
            id: "example_mod".into(),
            features: features.iter().map(|f| f.to_string()).collect(),
            ..ModMeta::default()
        }
    }

    fn lua() -> Option<&'static str> {
        ["luajit", "lua5.1", "lua"]
            .into_iter()
            .find(|exe| std::process::Command::new(exe).arg("-v").output().is_ok())
    }

    /// The game's Lua files from the local decompile, when it is there.
    fn game_file(rel: &str) -> Option<String> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../decompile");
        std::fs::read(root.join(rel))
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    #[test]
    fn the_runtime_in_a_lua_interpreter() {
        let Some(lua) = lua() else {
            eprintln!("skipping: needs a Lua interpreter");
            return;
        };
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let out = std::process::Command::new(lua)
            .arg(dir.join("testdata/ordnance_harness.lua"))
            .arg(dir.join("src/patch/events_runtime.lua"))
            .arg(dir.join("src/patch/skills_runtime.lua"))
            .arg(dir.join("src/patch/ordnance_runtime.lua"))
            .arg(dir.join("src/patch/faction_runtime.lua"))
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success() && text.trim_end().ends_with("ok"), "ordnance harness failed:\n{text}");
    }

    #[test]
    fn the_skills_runtime_in_a_lua_interpreter() {
        let Some(lua) = lua() else {
            eprintln!("skipping: needs a Lua interpreter");
            return;
        };
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let out = std::process::Command::new(lua)
            .arg(dir.join("testdata/skills_harness.lua"))
            .arg(dir.join("src/patch/events_runtime.lua"))
            .arg(dir.join("src/patch/skills_runtime.lua"))
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success() && text.trim_end().ends_with("ok"), "skills harness failed:
{text}");
    }

    /// Each hook's lines are in their game file as many times as the hook says, and the file
    /// with the hook in each place still compiles.
    #[test]
    fn hooks_fit_the_games_code() {
        let mut checked = 0;
        for (_, _, target, pattern, payload, after, times) in HOOKS {
            let Some(text) = game_file(target) else {
                eprintln!("decompile/ not found; skipped");
                return;
            };
            let lines: Vec<&str> = text.lines().collect();
            let want: Vec<&str> = pattern.lines().map(str::trim).collect();
            let at: Vec<usize> = (0..lines.len().saturating_sub(want.len() - 1))
                .filter(|&i| (0..want.len()).all(|k| lines[i + k].trim() == want[k]))
                .collect();
            assert_eq!(at.len(), times, "{target}: '{pattern}' appears {} times", at.len());
            let mut patched: Vec<&str> = lines.clone();
            // From the last place back, so the earlier places keep their line numbers.
            for &start in at.iter().rev() {
                let place = if after { start + want.len() } else { start };
                patched.insert(place, payload);
            }
            if let Some(lua) = lua() {
                let dir = tempfile::tempdir().unwrap();
                let file = dir.path().join("patched.lua");
                std::fs::write(&file, patched.join("\n")).unwrap();
                let luac = lua.replace("lua", "luac");
                if let Ok(out) = std::process::Command::new(&luac).arg("-p").arg(&file).output() {
                    assert!(
                        out.status.success(),
                        "{target} with the hook does not compile: {}",
                        String::from_utf8_lossy(&out.stderr)
                    );
                }
            }
            checked += 1;
        }
        assert_eq!(checked, 8);
    }

    /// The explosion presets carry the game's own numbers.
    #[test]
    fn presets_match_the_games_explosions() {
        let Some(game) = game_file("includes/urb_engine.lua") else {
            eprintln!("decompile/ not found; skipped");
            return;
        };
        // Only createExplosion's branches: other functions test the same tags.
        let from = game.find("function  createExplosion(tag, x, y, dad)").expect("createExplosion");
        let game = &game[from..];
        let runtime = include_str!("ordnance_runtime.lua");
        let preset_re = regex_lite::Regex::new(r#"(?s)(\w+) = \{ tag = "(\w+)",(.*?)\}"#).unwrap();
        let field_re = regex_lite::Regex::new(r"(\w+) = ([0-9.]+)").unwrap();
        let game_field_re =
            regex_lite::Regex::new(r"player\.explosions\[#player\.explosions\]\.(\w+) = ([0-9.]+)").unwrap();
        let else_re = regex_lite::Regex::new(r"\n[ \t]*else[ \t]*\r?\n").unwrap();
        let mut seen = 0;
        for c in preset_re.captures_iter(runtime) {
            let (name, tag, body) = (&c[1], &c[2], &c[3]);
            let start = game
                .find(&format!("(tag == \"{tag}\") then"))
                .unwrap_or_else(|| panic!("the game has no {tag}"));
            let rest = &game[start..];
            // The branch ends at the next one, or at the game's catch-all `else` (the last
            // branch, artyExplosion, runs into it).
            let next = rest[1..].find("elseif(tag ==").map_or(rest.len(), |e| e + 1);
            let catch_all = else_re.find(rest).map_or(rest.len(), |m| m.start());
            let end = next.min(catch_all);
            let block = &rest[..end];
            for f in field_re.captures_iter(body) {
                let key = &f[1];
                if matches!(key, "crater" | "sounds") {
                    continue;
                }
                // The last assignment is the one that holds (the game sets some twice).
                let want = game_field_re
                    .captures_iter(block)
                    .filter(|g| &g[1] == key)
                    .last()
                    .map(|g| g[2].to_string())
                    .unwrap_or_else(|| panic!("{name}: the game's {tag} has no {key}"));
                assert_eq!(&f[2], want, "{name}.{key} (the game's {tag})");
            }
            seen += 1;
        }
        assert_eq!(seen, 10);
    }

    #[test]
    fn hooks_only_for_a_mod_that_lists_grenades() {
        assert!(build_ordnance_patches(&[meta(&["events"])], false).is_empty());
        assert_eq!(build_ordnance_patches(&[meta(&["events"])], true).len(), 8, "a faction's ordnance");
        assert_eq!(build_ordnance_patches(&[meta(&["skills"])], false).len(), 8, "class skills alone");
        let patches = build_ordnance_patches(&[meta(&[]), meta(&["ordnance"])], false);
        assert_eq!(patches.len(), 8);
        let times: Vec<Option<usize>> = patches
            .iter()
            .map(|p| match p {
                Patch::Pattern(x) => x.times,
                _ => None,
            })
            .collect();
        assert_eq!(
            times,
            [Some(1), Some(1), Some(1), Some(1), Some(1), Some(2), Some(1), Some(1)],
            "the rifle grenade's two places"
        );
    }
}
