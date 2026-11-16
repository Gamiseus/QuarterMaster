use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

/// A parsed Lua value from a `ser.lua` file (zero code execution).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SerValue {
    Nil,
    Bool(bool),
    Number(f64),
    String(String),
    Ref(usize),
    Table(SerTable),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SerTable {
    pub array: Vec<SerValue>,
    pub map: HashMap<String, SerValue>,
}

impl SerTable {
    pub fn get_str<'a>(&'a self, key: &str, memo: &'a HashMap<usize, SerTable>) -> Option<&'a str> {
        match self.resolve(key, memo)? {
            SerValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn get_f64(&self, key: &str, memo: &HashMap<usize, SerTable>) -> Option<f64> {
        match self.resolve(key, memo)? {
            SerValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn get_i64(&self, key: &str, memo: &HashMap<usize, SerTable>) -> Option<i64> {
        self.get_f64(key, memo).map(|n| n as i64)
    }

    pub fn get_bool(&self, key: &str, memo: &HashMap<usize, SerTable>) -> Option<bool> {
        match self.resolve(key, memo)? {
            SerValue::Bool(b) => Some(*b),
            SerValue::Number(n) => Some(*n != 0.0),
            _ => None,
        }
    }

    pub fn get_table<'a>(
        &'a self,
        key: &str,
        memo: &'a HashMap<usize, SerTable>,
    ) -> Option<&'a SerTable> {
        match self.map.get(key)? {
            SerValue::Ref(idx) => memo.get(idx),
            SerValue::Table(t) => Some(t),
            _ => None,
        }
    }

    pub fn resolve<'a>(
        &'a self,
        key: &str,
        _memo: &'a HashMap<usize, SerTable>,
    ) -> Option<&'a SerValue> {
        let val = self.map.get(key)?;
        match val {
            SerValue::Ref(_idx) => Some(val),
            other => Some(other),
        }
    }
}

/// Complete parsed representation of a `ser.lua` file (`_[n]={...}` memo tables + root table).
#[derive(Debug, Clone, Default)]
pub struct SerDocument {
    pub memo: HashMap<usize, SerTable>,
    pub root: SerTable,
}

impl SerDocument {
    pub fn resolve_table<'a>(&'a self, val: &'a SerValue) -> Option<&'a SerTable> {
        match val {
            SerValue::Ref(idx) => self.memo.get(idx),
            SerValue::Table(t) => Some(t),
            _ => None,
        }
    }

    /// Parse any `ser.lua` string safely without executing Lua.
    /// Also tolerates the legacy `"return local _={}"` prefix produced by earlier GMenu versions.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let mut doc = SerDocument::default();

        for (line_idx, raw_line) in raw.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with("--") {
                continue;
            }
            // Tolerate `local _={}` or legacy `return local _={}`
            if line == "local _={}" || line == "return local _={}" {
                continue;
            }

            if let Some(rest) = line.strip_prefix("return ") {
                let rest = rest.trim();
                if rest == "_[0]" {
                    if let Some(t0) = doc.memo.remove(&0) {
                        doc.root = t0;
                    }
                } else if rest.starts_with('{') {
                    let mut parser = LineParser::new(rest);
                    doc.root = parser
                        .parse_table()
                        .map_err(|e| format!("Line {}: {e}", line_idx + 1))?;
                }
                continue;
            }

            if let Some(rest) = line.strip_prefix("_[") {
                if let Some((idx_str, after_bracket)) = rest.split_once(']') {
                    let idx: usize = idx_str.trim().parse().map_err(|_| {
                        format!("Line {}: invalid memo index '{}'", line_idx + 1, idx_str)
                    })?;
                    let after_bracket = after_bracket.trim();
                    if let Some(rhs) = after_bracket.strip_prefix('=') {
                        let rhs = rhs.trim();
                        if rhs.starts_with('{') {
                            let mut parser = LineParser::new(rhs);
                            let tbl = parser
                                .parse_table()
                                .map_err(|e| format!("Line {}: {e}", line_idx + 1))?;
                            doc.memo.insert(idx, tbl);
                        }
                    }
                }
            }
        }

        Ok(doc)
    }

    /// Where each entry of the root table sits in `raw`, in file order (Phase E D86). The
    /// root table is the `return {...}` line, or the `_[0]={...}` line when the file ends in
    /// `return _[0]`. Byte offsets are into `raw`.
    pub fn root_entry_spans(raw: &str) -> Result<Vec<RootEntry>, String> {
        let mut root_line: Option<(usize, &str)> = None;
        let mut memo_zero: Option<(usize, &str)> = None;
        let mut returns_memo_zero = false;
        let base = raw.as_ptr() as usize;
        for raw_line in raw.lines() {
            let line = raw_line.trim();
            if let Some(rest) = line.strip_prefix("return ") {
                let trimmed = rest.trim();
                if trimmed == "_[0]" {
                    returns_memo_zero = true;
                } else if trimmed.starts_with('{') {
                    root_line = Some((trimmed.as_ptr() as usize - base, trimmed));
                }
            } else if let Some(rest) = line.strip_prefix("_[0]") {
                if let Some(rhs) = rest.trim_start().strip_prefix('=') {
                    let rhs = rhs.trim();
                    if rhs.starts_with('{') {
                        memo_zero = Some((rhs.as_ptr() as usize - base, rhs));
                    }
                }
            }
        }
        let (offset, text) = match (root_line, returns_memo_zero, memo_zero) {
            (Some(r), _, _) => r,
            (None, true, Some(m)) => m,
            _ => return Err("The save has no root table".to_string()),
        };
        let mut parser = LineParser::new(text);
        let mut entries = parser.root_entries()?;
        for e in &mut entries {
            e.entry = e.entry.start + offset..e.entry.end + offset;
        }
        Ok(entries)
    }

    /// `raw` without the root-table entries named in `keys`: each entry is cut out with one
    /// separator and every other byte stays as it is (Phase E D86). Returns the new text and
    /// the keys that were found and removed. Keys set by a later `_[0].key=...` line (a
    /// cyclic value) are not root-table entries and are left alone.
    pub fn remove_root_keys(raw: &str, keys: &[&str]) -> Result<(String, Vec<String>), String> {
        let entries = Self::root_entry_spans(raw)?;
        let gone: Vec<bool> = entries
            .iter()
            .map(|e| e.key.as_deref().is_some_and(|k| keys.contains(&k)))
            .collect();
        let mut cut = vec![false; raw.len()];
        let mut mark = |range: Range<usize>| {
            for b in cut.iter_mut().take(range.end).skip(range.start) {
                *b = true;
            }
        };
        let mut removed = Vec::new();
        for (i, e) in entries.iter().enumerate() {
            if !gone.get(i).copied().unwrap_or(false) {
                continue;
            }
            if let Some(k) = &e.key {
                removed.push(k.clone());
            }
            match entries.get(i + 1) {
                // Up to the next entry: the entry and the separator after it.
                Some(next) => mark(e.entry.start..next.entry.start),
                // The last entry: also the separator before it, back to the last kept entry.
                None => {
                    let kept_end = entries
                        .iter()
                        .zip(&gone)
                        .take(i)
                        .filter(|(_, g)| !**g)
                        .map(|(k, _)| k.entry.end)
                        .next_back();
                    let from = match (kept_end, entries.first()) {
                        (Some(end), _) => end,
                        (None, Some(first)) => first.entry.start,
                        (None, None) => e.entry.start,
                    };
                    mark(from..e.entry.end);
                }
            }
        }
        let bytes: Vec<u8> = raw
            .bytes()
            .zip(&cut)
            .filter(|(_, c)| !**c)
            .map(|(b, _)| b)
            .collect();
        let text =
            String::from_utf8(bytes).map_err(|_| "The edit split a character".to_string())?;
        Ok((text, removed))
    }
}

/// One entry of a save's root table and where it sits in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootEntry {
    /// The key; None for a positional entry.
    pub key: Option<String>,
    /// From the first byte of the key (or value) to the last byte of the value.
    pub entry: Range<usize>,
}

struct LineParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> LineParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            bytes: input.as_bytes(),
            pos: 0,
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn parse_table(&mut self) -> Result<SerTable, String> {
        self.skip_ws();
        if self.peek() != Some(b'{') {
            return Err("Expected '{'".to_string());
        }
        self.pos += 1;

        let mut table = SerTable::default();
        loop {
            self.skip_ws();
            match self.peek() {
                None => return Err("Unexpected end of input inside table".to_string()),
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                Some(b',') | Some(b';') => {
                    self.pos += 1;
                    continue;
                }
                _ => {}
            }

            // Check if this entry is `[key]=val`, `ident=val`, or a positional array element
            if self.peek() == Some(b'[') {
                self.pos += 1;
                let key_val = self.parse_value()?;
                self.skip_ws();
                if self.peek() != Some(b']') {
                    return Err("Expected ']' after bracketed key".to_string());
                }
                self.pos += 1;
                self.skip_ws();
                if self.peek() != Some(b'=') {
                    return Err("Expected '=' after bracketed key".to_string());
                }
                self.pos += 1;
                let val = self.parse_value()?;
                let key_str = match key_val {
                    SerValue::String(s) => s,
                    SerValue::Number(n) => format!("{n}"),
                    SerValue::Bool(b) => format!("{b}"),
                    _ => continue,
                };
                table.map.insert(key_str, val);
            } else {
                let saved_pos = self.pos;
                if let Some(ident) = self.try_parse_ident_key() {
                    let val = self.parse_value()?;
                    table.map.insert(ident, val);
                } else {
                    self.pos = saved_pos;
                    let val = self.parse_value()?;
                    table.array.push(val);
                }
            }
        }

        Ok(table)
    }

    /// The entries of the table at the start of the input, with their byte ranges in it. The
    /// same walk as `parse_table`, for the outermost table only.
    fn root_entries(&mut self) -> Result<Vec<RootEntry>, String> {
        self.skip_ws();
        if self.peek() != Some(b'{') {
            return Err("Expected '{'".to_string());
        }
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => return Err("Unexpected end of input inside table".to_string()),
                Some(b'}') => break,
                Some(b',') | Some(b';') => {
                    self.pos += 1;
                    continue;
                }
                _ => {}
            }
            let start = self.pos;
            let key = if self.peek() == Some(b'[') {
                self.pos += 1;
                let key_val = self.parse_value()?;
                self.skip_ws();
                if self.peek() != Some(b']') {
                    return Err("Expected ']' after bracketed key".to_string());
                }
                self.pos += 1;
                self.skip_ws();
                if self.peek() != Some(b'=') {
                    return Err("Expected '=' after bracketed key".to_string());
                }
                self.pos += 1;
                self.parse_value()?;
                match key_val {
                    SerValue::String(s) => Some(s),
                    SerValue::Number(n) => Some(format!("{n}")),
                    SerValue::Bool(b) => Some(format!("{b}")),
                    _ => None,
                }
            } else if let Some(ident) = self.try_parse_ident_key() {
                self.parse_value()?;
                Some(ident)
            } else {
                self.pos = start;
                self.parse_value()?;
                None
            };
            out.push(RootEntry {
                key,
                entry: start..self.pos,
            });
        }
        Ok(out)
    }

    /// Checks for `ident = ` (not `_[idx]`). Advances past `=` if matched.
    fn try_parse_ident_key(&mut self) -> Option<String> {
        self.skip_ws();
        let start = self.pos;
        if start >= self.bytes.len() {
            return None;
        }
        let first = self.bytes[start];
        if !(first.is_ascii_alphabetic() || first == b'_') {
            return None;
        }
        while self.pos < self.bytes.len()
            && (self.bytes[self.pos].is_ascii_alphanumeric() || self.bytes[self.pos] == b'_')
        {
            self.pos += 1;
        }
        let ident = std::str::from_utf8(&self.bytes[start..self.pos])
            .ok()?
            .to_string();
        self.skip_ws();
        if self.peek() == Some(b'=') {
            self.pos += 1;
            Some(ident)
        } else {
            None
        }
    }

    fn parse_value(&mut self) -> Result<SerValue, String> {
        self.skip_ws();
        match self.peek() {
            None => Err("Unexpected end of input while parsing value".to_string()),
            Some(b'{') => Ok(SerValue::Table(self.parse_table()?)),
            Some(b'"') | Some(b'\'') => Ok(SerValue::String(self.parse_string()?)),
            Some(b'_') if self.bytes.get(self.pos + 1) == Some(&b'[') => {
                self.pos += 2;
                let start = self.pos;
                while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
                let idx_str = std::str::from_utf8(&self.bytes[start..self.pos])
                    .map_err(|_| "Invalid UTF-8 in ref index".to_string())?;
                let idx: usize = idx_str
                    .parse()
                    .map_err(|_| format!("Invalid ref index '{idx_str}'"))?;
                if self.peek() != Some(b']') {
                    return Err("Expected ']' after _[idx".to_string());
                }
                self.pos += 1;
                Ok(SerValue::Ref(idx))
            }
            _ => {
                let start = self.pos;
                while self.pos < self.bytes.len() {
                    let b = self.bytes[self.pos];
                    if b == b',' || b == b'}' || b == b';' || b == b']' || b.is_ascii_whitespace() {
                        break;
                    }
                    self.pos += 1;
                }
                let token = std::str::from_utf8(&self.bytes[start..self.pos])
                    .map_err(|_| "Invalid token".to_string())?;
                match token {
                    "true" => Ok(SerValue::Bool(true)),
                    "false" => Ok(SerValue::Bool(false)),
                    "nil" => Ok(SerValue::Nil),
                    "1/0" => Ok(SerValue::Number(f64::INFINITY)),
                    "-1/0" => Ok(SerValue::Number(f64::NEG_INFINITY)),
                    "0/0" | "-(0/0)" => Ok(SerValue::Number(0.0)),
                    _ => {
                        if let Ok(n) = token.parse::<f64>() {
                            Ok(SerValue::Number(n))
                        } else {
                            // Tolerate `loadstring"..."` or unknown tokens by skipping
                            Ok(SerValue::String(token.to_string()))
                        }
                    }
                }
            }
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        let quote = self.bytes[self.pos];
        self.pos += 1;
        let mut raw_bytes = Vec::new();

        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            self.pos += 1;
            if b == quote {
                return Ok(String::from_utf8_lossy(&raw_bytes).into_owned());
            }
            if b == b'\\' {
                if self.pos >= self.bytes.len() {
                    break;
                }
                let esc = self.bytes[self.pos];
                if esc.is_ascii_digit() {
                    // Lua decimal byte escape `\ddd` (up to 3 digits, used by ser.lua for UTF-8 bytes >= 128)
                    let mut val: u16 = 0;
                    let mut count = 0;
                    while count < 3
                        && self.pos < self.bytes.len()
                        && self.bytes[self.pos].is_ascii_digit()
                    {
                        val = val * 10 + (self.bytes[self.pos] - b'0') as u16;
                        self.pos += 1;
                        count += 1;
                    }
                    if val <= 255 {
                        raw_bytes.push(val as u8);
                    }
                } else {
                    self.pos += 1;
                    match esc {
                        b'n' => raw_bytes.push(b'\n'),
                        b'r' => raw_bytes.push(b'\r'),
                        b't' => raw_bytes.push(b'\t'),
                        b'\\' => raw_bytes.push(b'\\'),
                        b'"' => raw_bytes.push(b'"'),
                        b'\'' => raw_bytes.push(b'\''),
                        other => raw_bytes.push(other),
                    }
                }
            } else {
                raw_bytes.push(b);
            }
        }

        Err("Unterminated string literal".to_string())
    }
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReinforcementCardSummary {
    pub id: i64,
    pub tier: i64,
    pub card_type: String,
    pub sub_type: String,
    pub weapon_name: String,
    pub amount: i64,
    pub cost: i64,
}

/// Authentic rank title lookups matching base game engine (urb_engine.lua).
pub fn campaign_rank_title(lvl: i64) -> &'static str {
    match lvl {
        0 => "Dirty civy",
        1 => "Cadet",
        2 => "2nd lt",
        3 => "1rst lt",
        4 => "Captain",
        5 => "Major",
        6 => "Lieutenant Colonel",
        7 => "Colonel",
        8 => "Brigadier General",
        9 => "Major General",
        10 => "Lieutenant General",
        11 => "General",
        12 => "Mud and Blood General",
        13.. => "Mud and Blood Grand Master",
        _ => "Dirty civy",
    }
}

pub fn classic_rank_title(lvl: i64) -> &'static str {
    match lvl {
        0 => "Balloon enthusiast",
        1 => "Squad Commander",
        2 => "Section Commander",
        3 => "Platoon Commander",
        4 => "Company Commander",
        5 => "Battalion Commander",
        6 => "Regiment Commander",
        7 => "Brigade Commander",
        8 => "Division Commander",
        9 => "Corps Commander",
        10 => "Army Commander",
        11 => "Army Group Commander",
        12 => "Grand Commander",
        13.. => "Grand Master Commander",
        _ => "Balloon enthusiast",
    }
}

pub fn soldier_rank_title(rank: Option<i64>, spec: &str) -> &'static str {
    if spec.eq_ignore_ascii_case("officer") {
        match rank {
            None => "Cadet",
            Some(0) => "Second Lieutenant",
            Some(1) => "First Lieutenant",
            Some(2) => "Captain",
            Some(3..) => "Major",
            _ => "Cadet",
        }
    } else {
        match rank {
            None => "Recruit",
            Some(0) => "Private",
            Some(1) => "Private First Class",
            Some(2) => "Corporal",
            Some(3..) => "Sergeant",
            _ => "Recruit",
        }
    }
}

pub fn trench_rank_title(rank: i64) -> &'static str {
    match rank {
        1 => "Soldier",
        2 => "Lance Corporal",
        3 => "Corporal",
        4 => "Senior Corporal",
        5 => "Sergeant",
        6 => "Staff Sergeant",
        7 => "Second Lieutenant",
        8 => "First Lieutenant",
        9 => "Captain",
        10 | 11 => "Major",
        12 => "Lieutenant Colonel",
        13 => "Colonel",
        14 => "Brigadier General",
        15 => "Major General",
        16.. => "General",
        _ => "Soldier",
    }
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
/// High-level defensive summary of a Mud and Blood save file (`saveMNB3.lua` or `saveMNB2.lua`).
/// Every field is optional/defaulted so partial or modded saves never fail to render.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SaveSummary {
    pub file_name: String,
    pub area_of_operations: String,
    pub german_commander: String,
    pub day: i64,
    pub rank: i64,
    pub experience: i64,
    pub tactical_points: i64,
    pub total_tp_spent: i64,
    pub command_points: i64,
    pub campaign_kills: i64,
    pub campaign_casualties: i64,
    pub win_streak: i64,
    pub missions_completed: i64,
    pub campaign_rank: i64,
    pub campaign_experience: i64,
    pub classic_rank: i64,
    pub classic_experience: i64,
    pub classic_total_kills: i64,
    pub classic_total_casualties: i64,
    pub rank_title: String,
    pub campaign_rank_title: String,
    pub trench_rank_title: String,
    pub classic_max_wave: i64,
    pub classic_current_wave: i64,
    pub trench_rank: i64,
    pub trench_xp: i64,
    pub trench_axis_kills: i64,
    pub trench_total_casualties: i64,
    pub trench_escalation: i64,
    pub free_planes: i64,
    pub trench_sectors: Vec<String>,
    pub reinforcement_cards: Vec<ReinforcementCardSummary>,
    pub deployed_squad: Vec<SoldierSummary>,
    pub reserve_soldiers: Vec<SoldierSummary>,
    pub fallen_soldiers_count: usize,
    pub ribbons_count: usize,
    /// Ribbon ids from `ribs`, in the order they were earned (`sprites/rib<N>.png`).
    pub ribbons: Vec<i64>,
    pub unlocked_classes: Vec<String>,
    pub class_unlock_trees: HashMap<String, Vec<String>>,
    pub active_battle_conditions: Vec<String>,
    pub map_buildings_count: usize,
    pub map_placeables_count: usize,
    pub map_depth_px: i64,
    pub fight_intensity_record: i64,
    pub mod_options: HashMap<String, f64>,
    pub qm_meta: Option<SaveQmMeta>,
    pub unknown_mod_keys: Vec<String>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SoldierSummary {
    pub id: i64,
    pub name: String,
    pub age: i64,
    pub rank: i64,
    pub rank_title: String,
    pub level: i64,
    pub xp: i64,
    pub kills: i64,
    pub hth_kills: i64,
    pub shots: i64,
    pub hits: i64,
    pub spots: i64,
    pub revive: i64,
    pub mediced: i64,
    pub skill: i64,
    pub health: f64,
    pub max_health: f64,
    pub morale: f64,
    pub max_morale: f64,
    pub speed: f64,
    pub detect_range: i64,
    pub mine_detect_range: i64,
    pub worth: i64,
    pub state: String,
    pub specialization: String,
    pub weapon_tag: String,
    pub weapon_action: String,
    pub weapon_ammo: i64,
    pub weapon_max_ammo: i64,
    pub weapon_damage: i64,
    pub weapon_range: i64,
    pub weapon_rof: i64,
    pub weapon_recoil: i64,
    pub weapon_jam: i64,
    pub equipment: Vec<String>,
    pub awards_count: usize,
    /// Icon ids of the soldier's awards (`sprites/award<N>.png`).
    pub award_icons: Vec<i64>,
    pub perks_count: usize,
    pub combat_time: f64,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
/// Embedded `_qm_meta` mod fingerprint stamped into `saveMNB3.lua` / `saveMNB2.lua` on save.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveQmMeta {
    pub profile_id: Option<String>,
    pub profile_name: Option<String>,
    pub timestamp: Option<String>,
    /// Map of `mod_id -> version` active when the save was written.
    pub mods: HashMap<String, String>,
    /// `mod_id -> save fields` that mod owned when the save was stamped (Phase E D85). Empty
    /// for a stamp from before Phase E.
    #[serde(default)]
    pub fields: HashMap<String, Vec<String>>,
}

impl SaveSummary {
    pub fn from_path(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read save file {}: {e}", path.display()))?;
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("saveMNB3.lua")
            .to_string();
        Self::from_ser_str(&file_name, &raw)
    }

    pub fn from_ser_str(file_name: &str, raw: &str) -> Result<Self, String> {
        let doc = SerDocument::parse(raw)?;
        let root = &doc.root;
        let memo = &doc.memo;

        let mut summary = SaveSummary {
            file_name: file_name.to_string(),
            area_of_operations: root
                .get_str("currentao", memo)
                .or_else(|| root.get_str("ao", memo))
                .unwrap_or("Unknown AO")
                .to_string(),
            german_commander: root
                .get_str("germCommanderName", memo)
                .unwrap_or("Unknown")
                .to_string(),
            day: root.get_i64("dayz", memo).unwrap_or(1),
            rank: root.get_i64("rank", memo).unwrap_or(1),
            experience: root.get_i64("experience", memo).unwrap_or(0),
            tactical_points: root.get_i64("tp", memo).unwrap_or(0),
            total_tp_spent: root.get_i64("tpSpentTotal", memo).unwrap_or(0),
            command_points: root.get_i64("command", memo).unwrap_or(0),
            campaign_kills: root.get_i64("kills", memo).unwrap_or(0),
            campaign_casualties: root.get_i64("casualty", memo).unwrap_or(0),
            win_streak: root.get_i64("winStreak", memo).unwrap_or(0),
            missions_completed: root.get_i64("missionCounter", memo).unwrap_or(0),
            // Mode fields, checked against decompile/includes (plan 2c Task 7). The game's main
            // menu shows "Campaign rank: rankTell(player.rank)" and "Classic rank:
            // mnb2RankTell(player.mnb2Rank)"; `mnb2*` is the classic (wave) mode.
            //
            // | Field                     | Game variable          | Where the game sets it            |
            // |---------------------------|------------------------|-----------------------------------|
            // | day                       | dayz                   | urb_engine.lua (+1 per day)       |
            // | campaign_rank / _title    | rank, rankTell()       | urb_engine.lua main menu          |
            // | campaign_experience       | experience             | campaign debrief                  |
            // | campaign_kills            | kills                  | urb_engine.lua, gameType 0        |
            // | campaign_casualties       | casualty               | urb_engine.lua                    |
            // | missions_completed        | missionCounter         | campaign                          |
            // | classic_rank / rank_title | mnb2Rank, mnb2RankTell | mnb2.lua debrief                  |
            // | classic_experience        | mnb2Experience         | mnb2.lua (score from waves)       |
            // | classic_total_kills       | mnb2TotalKills         | urb_engine.lua, gameType 1        |
            // | classic_total_casualties  | mnb2TotalCasualty      | urb_engine.lua                    |
            // | classic_max_wave / current| maxWave, currentWave   | mnb2.lua                          |
            // | trench_rank / _title      | trenchModeRank         | drawFiles.lua                     |
            // | trench_xp                 | trenchModeXP           | trench.lua                        |
            // | ribbons                   | ribs (ids, in order)   | bcChecks.lua checkRibCondition    |
            campaign_rank: root.get_i64("rank", memo).unwrap_or(1),
            campaign_experience: root.get_i64("experience", memo).unwrap_or(0),
            classic_rank: root.get_i64("mnb2Rank", memo).unwrap_or(1),
            classic_experience: root.get_i64("mnb2Experience", memo).unwrap_or(0),
            classic_total_kills: root.get_i64("mnb2TotalKills", memo).unwrap_or(0),
            classic_total_casualties: root.get_i64("mnb2TotalCasualty", memo).unwrap_or(0),
            classic_max_wave: root.get_i64("maxWave", memo).unwrap_or(0),
            classic_current_wave: root.get_i64("currentWave", memo).unwrap_or(0),
            trench_rank: root.get_i64("trenchModeRank", memo).unwrap_or(1),
            rank_title: classic_rank_title(root.get_i64("mnb2Rank", memo).unwrap_or(1)).to_string(),
            campaign_rank_title: campaign_rank_title(root.get_i64("rank", memo).unwrap_or(1))
                .to_string(),
            trench_rank_title: trench_rank_title(root.get_i64("trenchModeRank", memo).unwrap_or(1))
                .to_string(),
            trench_xp: root.get_i64("trenchModeXP", memo).unwrap_or(0),
            trench_axis_kills: root.get_i64("trenchAxisKills", memo).unwrap_or(0),
            trench_total_casualties: root.get_i64("trenchTotalCasualty", memo).unwrap_or(0),
            trench_escalation: root.get_i64("trenchEscalation", memo).unwrap_or(0),
            free_planes: root.get_i64("freePlane", memo).unwrap_or(0),
            map_depth_px: root
                .get_i64("endLine", memo)
                .or_else(|| root.get_i64("endline", memo))
                .unwrap_or(0),
            fight_intensity_record: root.get_i64("fightIntensityRecord", memo).unwrap_or(0),
            ..Default::default()
        };

        // Extract trench sector control states (`trench = _[33]`)
        if let Some(trench_tbl) = root.get_table("trench", memo) {
            for item in &trench_tbl.array {
                if let SerValue::String(s) = item {
                    summary.trench_sectors.push(s.clone());
                }
            }
        }

        // Extract available reinforcement cards (`trenchSkill = _[12]`)
        if let Some(ts_tbl) = root.get_table("trenchSkill", memo) {
            for item in &ts_tbl.array {
                if let Some(c_tbl) = doc.resolve_table(item) {
                    let c_type = c_tbl.get_str("type", memo).unwrap_or("nothing");
                    if c_type != "nothing" && !c_type.is_empty() {
                        summary.reinforcement_cards.push(ReinforcementCardSummary {
                            id: c_tbl.get_i64("id", memo).unwrap_or(0),
                            tier: c_tbl.get_i64("tier", memo).unwrap_or(1),
                            card_type: c_type.to_string(),
                            sub_type: c_tbl.get_str("subType", memo).unwrap_or("").to_string(),
                            weapon_name: c_tbl
                                .get_str("weaponName", memo)
                                .unwrap_or("standard")
                                .to_string(),
                            amount: c_tbl.get_i64("amount", memo).unwrap_or(1),
                            cost: c_tbl.get_i64("cost", memo).unwrap_or(1),
                        });
                    }
                }
            }
        }

        // Extract map buildings and placeables count
        if let Some(b_tbl) = root.get_table("buildings", memo) {
            summary.map_buildings_count = b_tbl.array.len() + b_tbl.map.len();
        }
        if let Some(p_tbl) = root.get_table("placeables", memo) {
            summary.map_placeables_count = p_tbl.array.len() + p_tbl.map.len();
        }

        // Extract active Battle Condition modifiers (`bcMod*`)
        for (k, v) in &root.map {
            if let Some(bc_name) = k.strip_prefix("bcMod") {
                if let SerValue::Number(n) = v {
                    if *n > 0.0 {
                        summary
                            .active_battle_conditions
                            .push(format!("{bc_name}: {}", *n as i64));
                    }
                }
            }
        }
        summary.active_battle_conditions.sort();

        // Extract deployedTroops
        if let Some(deployed_tbl) = root.get_table("deployedTroops", memo) {
            summary.deployed_squad = Self::extract_soldiers(deployed_tbl, &doc);
        }

        // Extract reserve soldiers from troopBin + ironMan.soldiers
        if let Some(bin_tbl) = root.get_table("troopBin", memo) {
            summary
                .reserve_soldiers
                .extend(Self::extract_soldiers(bin_tbl, &doc));
        }
        if let Some(im_tbl) = root.get_table("ironMan", memo) {
            if let Some(im_soldiers) = im_tbl.get_table("soldiers", memo) {
                summary
                    .reserve_soldiers
                    .extend(Self::extract_soldiers(im_soldiers, &doc));
            }
        }

        // Fallen soldiers count (cimetery)
        if let Some(cim_tbl) = root.get_table("cimetery", memo) {
            summary.fallen_soldiers_count = cim_tbl.array.len() + cim_tbl.map.len();
        }

        // Ribbons count
        if let Some(ribs_tbl) = root.get_table("ribs", memo) {
            summary.ribbons_count = ribs_tbl.array.len() + ribs_tbl.map.len();
            summary.ribbons = ribs_tbl
                .array
                .iter()
                .filter_map(|v| match v {
                    SerValue::Number(n) if n.fract() == 0.0 => Some(*n as i64),
                    _ => None,
                })
                .collect();
        }

        // Unlocked classes & weapon trees (`unlock = _[19]`)
        if let Some(unlock_tbl) = root.get_table("unlock", memo) {
            for (class_name, val) in &unlock_tbl.map {
                if let Some(t) = doc.resolve_table(val) {
                    let weapons: Vec<String> = t
                        .array
                        .iter()
                        .filter_map(|item| match item {
                            SerValue::String(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect();
                    if !weapons.is_empty() || !t.map.is_empty() {
                        summary.unlocked_classes.push(class_name.clone());
                        summary
                            .class_unlock_trees
                            .insert(class_name.clone(), weapons);
                    }
                } else if matches!(val, SerValue::Number(n) if *n > 0.0) {
                    summary.unlocked_classes.push(class_name.clone());
                }
            }
            summary.unlocked_classes.sort();
        }

        // Extract `option_*` keys (including custom mod options like `option_safeDestinationEnabled`)
        let vanilla_options = [
            "option_music",
            "option_volume",
            "option_fullScreen",
            "option_floatyText",
            "option_fps",
            "option_textCues",
            "option_maxParticles",
            "option_maxDecals",
        ];
        for (k, v) in &root.map {
            if k.starts_with("option_") {
                if let SerValue::Number(n) = v {
                    summary.mod_options.insert(k.clone(), *n);
                }
                if !vanilla_options.contains(&k.as_str()) {
                    summary.unknown_mod_keys.push(k.clone());
                }
            }
        }
        summary.unknown_mod_keys.sort();

        // Extract `_qm_meta` if stamped
        if let Some(qm_tbl) = root.get_table("_qm_meta", memo) {
            let mut mods = HashMap::new();
            if let Some(mods_tbl) = qm_tbl.get_table("mods", memo) {
                for (m_id, m_ver) in &mods_tbl.map {
                    if let SerValue::String(ver_s) = m_ver {
                        mods.insert(m_id.clone(), ver_s.clone());
                    }
                }
            }
            let mut fields: HashMap<String, Vec<String>> = HashMap::new();
            if let Some(fields_tbl) = qm_tbl.get_table("fields", memo) {
                for (m_id, list) in &fields_tbl.map {
                    let Some(list) = doc.resolve_table(list) else {
                        continue;
                    };
                    let mut names: Vec<String> = list
                        .array
                        .iter()
                        .filter_map(|v| match v {
                            SerValue::String(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect();
                    names.sort();
                    names.dedup();
                    fields.insert(m_id.clone(), names);
                }
            }
            summary.qm_meta = Some(SaveQmMeta {
                profile_id: qm_tbl.get_str("profile_id", memo).map(|s| s.to_string()),
                profile_name: qm_tbl.get_str("profile_name", memo).map(|s| s.to_string()),
                timestamp: qm_tbl.get_str("timestamp", memo).map(|s| s.to_string()),
                mods,
                fields,
            });
        }

        Ok(summary)
    }

    fn extract_soldiers(list_tbl: &SerTable, doc: &SerDocument) -> Vec<SoldierSummary> {
        let memo = &doc.memo;
        let mut out = Vec::new();
        for item in &list_tbl.array {
            if let Some(s_tbl) = doc.resolve_table(item) {
                let w_tbl = s_tbl.get_table("weapon", memo);
                let weapon_tag = w_tbl
                    .and_then(|w| w.get_str("tag", memo))
                    .unwrap_or("unarmed")
                    .to_string();
                let weapon_action = w_tbl
                    .and_then(|w| w.get_str("action", memo))
                    .unwrap_or("bolt")
                    .to_string();
                let weapon_ammo = w_tbl.and_then(|w| w.get_i64("ammo", memo)).unwrap_or(0);
                let weapon_max_ammo = w_tbl.and_then(|w| w.get_i64("maxAmmo", memo)).unwrap_or(0);
                let weapon_damage = w_tbl.and_then(|w| w.get_i64("damage", memo)).unwrap_or(0);
                let weapon_range = w_tbl.and_then(|w| w.get_i64("range", memo)).unwrap_or(0);
                let weapon_rof = w_tbl.and_then(|w| w.get_i64("rof", memo)).unwrap_or(0);
                let weapon_recoil = w_tbl.and_then(|w| w.get_i64("recoil", memo)).unwrap_or(0);
                let weapon_jam = w_tbl.and_then(|w| w.get_i64("jam", memo)).unwrap_or(0);
                let sol_rank = s_tbl.get_i64("rank", memo);
                let sol_spec = s_tbl.get_str("spec", memo).unwrap_or("");
                let sol_rank_title = soldier_rank_title(sol_rank, sol_spec).to_string();

                let awards_tbl = s_tbl.get_table("awards", memo);
                let awards_count = awards_tbl.map(|t| t.array.len() + t.map.len()).unwrap_or(0);
                // Each award is `{icon = N, ...}`; the game draws `sprites/award<N>.png`.
                let award_icons: Vec<i64> = awards_tbl
                    .map(|t| {
                        t.array
                            .iter()
                            .filter_map(|v| match v {
                                SerValue::Table(t) => Some(t),
                                SerValue::Ref(i) => memo.get(i),
                                _ => None,
                            })
                            .filter_map(|a| a.get_i64("icon", memo))
                            .collect()
                    })
                    .unwrap_or_default();
                let perks_count = s_tbl
                    .get_table("perks", memo)
                    .map(|t| t.array.len() + t.map.len())
                    .unwrap_or(0);

                let mut equipment = Vec::new();
                for (flag_key, label) in [
                    ("helmet", "Helmet"),
                    ("armor", "Body Armor"),
                    ("camo", "Camo Smock"),
                    ("medicBag", "Medic Kit"),
                    ("radio", "Field Radio"),
                    ("wireCutter", "Wire Cutters"),
                    ("toolBox", "Engy Toolbox"),
                    ("shovel", "Entrenching Shovel"),
                    ("observationKit", "Binoculars"),
                    ("detector", "Mine Detector"),
                    ("bazooka", "Bazooka"),
                    ("flamer", "Flamethrower"),
                    ("mortar", "Mortar"),
                    ("bipod", "Bipod"),
                ] {
                    if s_tbl.get_i64(flag_key, memo).unwrap_or(0) > 0 {
                        equipment.push(label.to_string());
                    }
                }
                let frags = s_tbl.get_i64("frags", memo).unwrap_or(0);
                if frags > 0 {
                    equipment.push(format!("{frags}x Frag"));
                }

                out.push(SoldierSummary {
                    id: s_tbl.get_i64("id", memo).unwrap_or(0),
                    name: s_tbl.get_str("name", memo).unwrap_or("Unknown").to_string(),
                    age: s_tbl.get_i64("age", memo).unwrap_or(20),
                    rank: sol_rank.unwrap_or(0),
                    rank_title: sol_rank_title,
                    level: s_tbl.get_i64("level", memo).unwrap_or(1),
                    xp: s_tbl.get_i64("xp", memo).unwrap_or(0),
                    kills: s_tbl.get_i64("kills", memo).unwrap_or(0),
                    hth_kills: s_tbl.get_i64("hthKills", memo).unwrap_or(0),
                    shots: s_tbl.get_i64("shots", memo).unwrap_or(0),
                    hits: s_tbl.get_i64("hits", memo).unwrap_or(0),
                    spots: s_tbl.get_i64("spots", memo).unwrap_or(0),
                    revive: s_tbl.get_i64("revive", memo).unwrap_or(0),
                    mediced: s_tbl.get_i64("mediced", memo).unwrap_or(0),
                    skill: s_tbl.get_i64("skill", memo).unwrap_or(30),
                    health: s_tbl.get_f64("health", memo).unwrap_or(100.0),
                    max_health: s_tbl.get_f64("maxHealth", memo).unwrap_or(100.0),
                    morale: s_tbl.get_f64("morale", memo).unwrap_or(30.0),
                    max_morale: s_tbl.get_f64("maxMorale", memo).unwrap_or(100.0),
                    speed: s_tbl.get_f64("speed", memo).unwrap_or(85.0),
                    detect_range: s_tbl.get_i64("detectRange", memo).unwrap_or(1000),
                    mine_detect_range: s_tbl.get_i64("mineDetectRange", memo).unwrap_or(200),
                    worth: s_tbl.get_i64("worth", memo).unwrap_or(3),
                    state: s_tbl.get_str("state", memo).unwrap_or("idle").to_string(),
                    specialization: s_tbl.get_str("spec", memo).unwrap_or("").to_string(),
                    weapon_tag,
                    weapon_action,
                    weapon_ammo,
                    weapon_max_ammo,
                    weapon_damage,
                    weapon_range,
                    weapon_rof,
                    weapon_recoil,
                    weapon_jam,
                    equipment,
                    awards_count,
                    award_icons,
                    perks_count,
                    combat_time: s_tbl.get_f64("combatTime", memo).unwrap_or(0.0),
                });
            }
        }
        out
    }

    pub fn to_markdown_dossier(&self, slot_id: &str) -> String {
        let mut md = String::new();
        md.push_str(&format!(
            "# QuarterMaster Tactical Save Dossier — Slot `{slot_id}` (`{}`)\n\n",
            self.file_name
        ));
        md.push_str(&format!(
            "- **Area of Operations:** {} (Day {})\n",
            self.area_of_operations, self.day
        ));
        md.push_str(&format!(
            "- **Opposing German Commander:** {}\n",
            self.german_commander
        ));
        md.push_str(&format!(
            "- **Campaign Progress:** Rank {} | {} XP | {} TP ({} Total TP Spent) | {} Command Points\n",
            self.rank, self.experience, self.tactical_points, self.total_tp_spent, self.command_points
        ));
        md.push_str(&format!(
            "- **Combat Record:** {} Kills | {} KIA | Win Streak: {} | Missions: {}\n",
            self.campaign_kills, self.campaign_casualties, self.win_streak, self.missions_completed
        ));
        md.push_str(&format!(
            "- **Classic & Trench Mode:** Classic Rank {} (Max Wave {}, {} Kills) | Trench Rank {} ({} XP, {} Axis Kills)\n\n",
            self.classic_rank,
            self.classic_max_wave,
            self.classic_total_kills,
            self.trench_rank,
            self.trench_xp,
            self.trench_axis_kills
        ));
        md.push_str("## Deployed Squad Roster\n\n");
        md.push_str("| Soldier | Age | Rank | Lvl | Weapon | Ammo | HP | Morale | Skill | Kills | Equipment |\n");
        md.push_str("|---|---|---|---|---|---|---|---|---|---|---|\n");
        for s in &self.deployed_squad {
            md.push_str(&format!(
                "| **{}** | {} | {} | {} | `{}` ({} dmg, {}m) | {}/{} | {:.0}/{:.0} | {:.0}/{:.0} | {} | {} | {} |\n",
                s.name,
                s.age,
                s.rank,
                s.level,
                s.weapon_tag,
                s.weapon_damage,
                s.weapon_range,
                s.weapon_ammo,
                s.weapon_max_ammo,
                s.health,
                s.max_health,
                s.morale,
                s.max_morale,
                s.skill,
                s.kills,
                if s.equipment.is_empty() {
                    "Standard".to_string()
                } else {
                    s.equipment.join(", ")
                }
            ));
        }
        md
    }
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
/// Diff between two `SaveSummary` snapshots (e.g., comparing two save profiles or a save vs its backup).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SaveComparison {
    pub ao_a: String,
    pub ao_b: String,
    pub day_a: i64,
    pub day_b: i64,
    pub day_delta: i64,
    pub tp_a: i64,
    pub tp_b: i64,
    pub tp_delta: i64,
    pub rank_a: i64,
    pub rank_b: i64,
    pub rank_delta: i64,
    pub experience_a: i64,
    pub experience_b: i64,
    pub experience_delta: i64,
    pub campaign_kills_a: i64,
    pub campaign_kills_b: i64,
    pub campaign_kills_delta: i64,
    pub campaign_casualties_a: i64,
    pub campaign_casualties_b: i64,
    pub campaign_casualties_delta: i64,
    pub classic_max_wave_a: i64,
    pub classic_max_wave_b: i64,
    pub classic_max_wave_delta: i64,
    pub classic_kills_delta: i64,
    pub soldiers_only_in_a: Vec<String>,
    pub soldiers_only_in_b: Vec<String>,
    pub added_soldiers: Vec<String>,
    pub lost_soldiers: Vec<String>,
    pub promoted_soldiers: Vec<String>,
    pub added_mods: Vec<String>,
    pub removed_mods: Vec<String>,
}

pub fn compare_saves(older: &SaveSummary, newer: &SaveSummary) -> SaveComparison {
    let mut cmp = SaveComparison {
        ao_a: older.area_of_operations.clone(),
        ao_b: newer.area_of_operations.clone(),
        day_a: older.day,
        day_b: newer.day,
        day_delta: newer.day - older.day,
        tp_a: older.tactical_points,
        tp_b: newer.tactical_points,
        tp_delta: newer.tactical_points - older.tactical_points,
        rank_a: older.rank,
        rank_b: newer.rank,
        rank_delta: newer.rank - older.rank,
        experience_a: older.experience,
        experience_b: newer.experience,
        experience_delta: newer.experience - older.experience,
        campaign_kills_a: older.campaign_kills,
        campaign_kills_b: newer.campaign_kills,
        campaign_kills_delta: newer.campaign_kills - older.campaign_kills,
        campaign_casualties_a: older.campaign_casualties,
        campaign_casualties_b: newer.campaign_casualties,
        campaign_casualties_delta: newer.campaign_casualties - older.campaign_casualties,
        classic_max_wave_a: older.classic_max_wave,
        classic_max_wave_b: newer.classic_max_wave,
        classic_max_wave_delta: newer.classic_max_wave - older.classic_max_wave,
        classic_kills_delta: newer.classic_total_kills - older.classic_total_kills,
        ..Default::default()
    };

    let old_map: HashMap<&str, &SoldierSummary> = older
        .deployed_squad
        .iter()
        .map(|s| (s.name.as_str(), s))
        .collect();
    let new_map: HashMap<&str, &SoldierSummary> = newer
        .deployed_squad
        .iter()
        .map(|s| (s.name.as_str(), s))
        .collect();

    for (name, s_new) in &new_map {
        if let Some(s_old) = old_map.get(name) {
            if s_new.rank > s_old.rank || s_new.level > s_old.level {
                cmp.promoted_soldiers.push(format!(
                    "{} (Rank {} -> {}, Kills {} -> {})",
                    name, s_old.rank, s_new.rank, s_old.kills, s_new.kills
                ));
            }
        } else {
            cmp.soldiers_only_in_b.push((*name).to_string());
            cmp.added_soldiers.push((*name).to_string());
        }
    }

    for name in old_map.keys() {
        if !new_map.contains_key(name) {
            cmp.soldiers_only_in_a.push((*name).to_string());
            cmp.lost_soldiers.push((*name).to_string());
        }
    }

    cmp.soldiers_only_in_a.sort();
    cmp.soldiers_only_in_b.sort();
    cmp.added_soldiers.sort();
    cmp.lost_soldiers.sort();
    cmp.promoted_soldiers.sort();

    let empty_mods = HashMap::new();
    let old_mods = older
        .qm_meta
        .as_ref()
        .map(|m| &m.mods)
        .unwrap_or(&empty_mods);
    let new_mods = newer
        .qm_meta
        .as_ref()
        .map(|m| &m.mods)
        .unwrap_or(&empty_mods);

    for m in new_mods.keys() {
        if !old_mods.contains_key(m) {
            cmp.added_mods.push(m.clone());
        }
    }
    for m in old_mods.keys() {
        if !new_mods.contains_key(m) {
            cmp.removed_mods.push(m.clone());
        }
    }
    cmp.added_mods.sort();
    cmp.removed_mods.sort();

    cmp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ser_lua_save_with_utf8_escapes_and_soldiers() {
        let sample = r#"local _={}
_[3]={ammo=5,maxAmmo=5,tag="springfield",damage=100}
_[2]={id=101,name="W. Haddon",age=20,rank=2,level=3,xp=450,kills=12,skill=45,health=100,maxHealth=100,morale=80,maxMorale=100,spec="sniper",weapon=_[3]}
_[1]={_[2]}
return {currentao="Omaha Beach",germCommanderName="A. M\195\188ller",dayz=4,rank=3,experience=1200,tp=15,kills=42,casualty=3,deployedTroops=_[1],option_safeDestinationEnabled=1,option_squadLimitThreshold=8}"#;

        let summary = SaveSummary::from_ser_str("saveMNB3.lua", sample).unwrap();
        assert_eq!(summary.area_of_operations, "Omaha Beach");
        assert_eq!(summary.german_commander, "A. Müller");
        assert_eq!(summary.day, 4);
        assert_eq!(summary.rank, 3);
        assert_eq!(summary.deployed_squad.len(), 1);
        assert_eq!(summary.deployed_squad[0].name, "W. Haddon");
        assert_eq!(summary.deployed_squad[0].weapon_tag, "springfield");
        assert_eq!(summary.deployed_squad[0].kills, 12);
        assert!(summary
            .unknown_mod_keys
            .contains(&"option_safeDestinationEnabled".to_string()));
    }

    #[test]
    fn test_compare_saves_identical_zero_deltas() {
        let mut save = SaveSummary {
            area_of_operations: "Omaha Beach".to_string(),
            day: 5,
            rank: 3,
            experience: 1000,
            tactical_points: 20,
            campaign_kills: 50,
            campaign_casualties: 2,
            classic_max_wave: 15,
            ..Default::default()
        };
        save.deployed_squad.push(SoldierSummary {
            name: "J. Miller".to_string(),
            rank: 2,
            level: 3,
            kills: 10,
            ..Default::default()
        });

        let cmp = compare_saves(&save, &save);
        assert_eq!(cmp.ao_a, "Omaha Beach");
        assert_eq!(cmp.ao_b, "Omaha Beach");
        assert_eq!(cmp.day_a, 5);
        assert_eq!(cmp.day_b, 5);
        assert_eq!(cmp.day_delta, 0);
        assert_eq!(cmp.tp_a, 20);
        assert_eq!(cmp.tp_b, 20);
        assert_eq!(cmp.tp_delta, 0);
        assert_eq!(cmp.rank_a, 3);
        assert_eq!(cmp.rank_b, 3);
        assert_eq!(cmp.rank_delta, 0);
        assert_eq!(cmp.experience_a, 1000);
        assert_eq!(cmp.experience_b, 1000);
        assert_eq!(cmp.experience_delta, 0);
        assert_eq!(cmp.campaign_kills_a, 50);
        assert_eq!(cmp.campaign_kills_b, 50);
        assert_eq!(cmp.campaign_kills_delta, 0);
        assert_eq!(cmp.campaign_casualties_a, 2);
        assert_eq!(cmp.campaign_casualties_b, 2);
        assert_eq!(cmp.campaign_casualties_delta, 0);
        assert_eq!(cmp.classic_max_wave_a, 15);
        assert_eq!(cmp.classic_max_wave_b, 15);
        assert_eq!(cmp.classic_max_wave_delta, 0);
        assert!(cmp.soldiers_only_in_a.is_empty());
        assert!(cmp.soldiers_only_in_b.is_empty());
        assert!(cmp.promoted_soldiers.is_empty());
        assert!(cmp.added_mods.is_empty());
        assert!(cmp.removed_mods.is_empty());
    }

    #[test]
    fn test_compare_saves_different_metrics_squad_and_mods() {
        let mut save_a = SaveSummary {
            area_of_operations: "Cherbourg".to_string(),
            day: 2,
            rank: 1,
            experience: 200,
            tactical_points: 5,
            campaign_kills: 10,
            campaign_casualties: 1,
            classic_max_wave: 5,
            ..Default::default()
        };
        save_a.deployed_squad.push(SoldierSummary {
            name: "J. Miller".to_string(),
            rank: 1,
            level: 1,
            kills: 5,
            ..Default::default()
        });
        save_a.deployed_squad.push(SoldierSummary {
            name: "R. Caparzo".to_string(),
            rank: 1,
            level: 1,
            kills: 2,
            ..Default::default()
        });

        let mut mods_a = HashMap::new();
        mods_a.insert("mod_alpha".to_string(), "1.0.0".to_string());
        save_a.qm_meta = Some(SaveQmMeta {
            mods: mods_a,
            ..Default::default()
        });

        let mut save_b = SaveSummary {
            area_of_operations: "Carentan".to_string(),
            day: 7,
            rank: 3,
            experience: 1500,
            tactical_points: 25,
            campaign_kills: 45,
            campaign_casualties: 3,
            classic_max_wave: 12,
            ..Default::default()
        };
        save_b.deployed_squad.push(SoldierSummary {
            name: "J. Miller".to_string(),
            rank: 2,
            level: 3,
            kills: 18,
            ..Default::default()
        });
        save_b.deployed_squad.push(SoldierSummary {
            name: "T. Ryan".to_string(),
            rank: 1,
            level: 2,
            kills: 7,
            ..Default::default()
        });

        let mut mods_b = HashMap::new();
        mods_b.insert("mod_beta".to_string(), "2.0.0".to_string());
        save_b.qm_meta = Some(SaveQmMeta {
            mods: mods_b,
            ..Default::default()
        });

        let cmp = compare_saves(&save_a, &save_b);
        assert_eq!(cmp.ao_a, "Cherbourg");
        assert_eq!(cmp.ao_b, "Carentan");
        assert_eq!(cmp.day_a, 2);
        assert_eq!(cmp.day_b, 7);
        assert_eq!(cmp.day_delta, 5);
        assert_eq!(cmp.tp_a, 5);
        assert_eq!(cmp.tp_b, 25);
        assert_eq!(cmp.tp_delta, 20);
        assert_eq!(cmp.rank_a, 1);
        assert_eq!(cmp.rank_b, 3);
        assert_eq!(cmp.rank_delta, 2);
        assert_eq!(cmp.experience_a, 200);
        assert_eq!(cmp.experience_b, 1500);
        assert_eq!(cmp.experience_delta, 1300);
        assert_eq!(cmp.campaign_kills_a, 10);
        assert_eq!(cmp.campaign_kills_b, 45);
        assert_eq!(cmp.campaign_kills_delta, 35);
        assert_eq!(cmp.campaign_casualties_a, 1);
        assert_eq!(cmp.campaign_casualties_b, 3);
        assert_eq!(cmp.campaign_casualties_delta, 2);
        assert_eq!(cmp.classic_max_wave_a, 5);
        assert_eq!(cmp.classic_max_wave_b, 12);
        assert_eq!(cmp.classic_max_wave_delta, 7);

        assert_eq!(cmp.soldiers_only_in_a, vec!["R. Caparzo".to_string()]);
        assert_eq!(cmp.soldiers_only_in_b, vec!["T. Ryan".to_string()]);
        assert_eq!(cmp.promoted_soldiers.len(), 1);
        assert!(cmp.promoted_soldiers[0].contains("J. Miller"));

        assert_eq!(cmp.added_mods, vec!["mod_beta".to_string()]);
        assert_eq!(cmp.removed_mods, vec!["mod_alpha".to_string()]);
    }

    #[test]
    fn test_verifies_authentic_base_game_ranks() {
        // Classic ranks (rankTell)
        assert_eq!(campaign_rank_title(0), "Dirty civy");
        assert_eq!(campaign_rank_title(1), "Cadet");
        assert_eq!(campaign_rank_title(2), "2nd lt");
        assert_eq!(campaign_rank_title(3), "1rst lt");
        assert_eq!(campaign_rank_title(4), "Captain");
        assert_eq!(campaign_rank_title(13), "Mud and Blood Grand Master");

        // Campaign ranks (mnb2RankTell)
        assert_eq!(classic_rank_title(0), "Balloon enthusiast");
        assert_eq!(classic_rank_title(1), "Squad Commander");
        assert_eq!(classic_rank_title(2), "Section Commander");
        assert_eq!(classic_rank_title(3), "Platoon Commander");
        assert_eq!(classic_rank_title(4), "Company Commander");
        assert_eq!(classic_rank_title(13), "Grand Master Commander");

        // Deployed Soldier ranks (soldierRankTell)
        // Enlisted
        assert_eq!(soldier_rank_title(None, "rifleman"), "Recruit");
        assert_eq!(soldier_rank_title(Some(0), "rifleman"), "Private");
        assert_eq!(
            soldier_rank_title(Some(1), "rifleman"),
            "Private First Class"
        );
        assert_eq!(soldier_rank_title(Some(2), "sniper"), "Corporal");
        assert_eq!(soldier_rank_title(Some(3), "gunner"), "Sergeant");

        // Officer
        assert_eq!(soldier_rank_title(None, "officer"), "Cadet");
        assert_eq!(soldier_rank_title(Some(0), "officer"), "Second Lieutenant");
        assert_eq!(soldier_rank_title(Some(1), "officer"), "First Lieutenant");
        assert_eq!(soldier_rank_title(Some(2), "officer"), "Captain");
        assert_eq!(soldier_rank_title(Some(3), "officer"), "Major");

        // Trench ranks (trenchRankTell)
        assert_eq!(trench_rank_title(1), "Soldier");
        assert_eq!(trench_rank_title(2), "Lance Corporal");
        assert_eq!(trench_rank_title(5), "Sergeant");
        assert_eq!(trench_rank_title(16), "General");
    }

    #[test]
    fn parser_reads_ribbon_ids() {
        let sample = r#"local _={}
_[1]={1,4,61}
return {ribs=_[1]}"#;
        let summary = SaveSummary::from_ser_str("saveMNB3.lua", sample).unwrap();
        assert_eq!(summary.ribbons, vec![1, 4, 61]);
        assert_eq!(summary.ribbons_count, 3);
    }

    #[test]
    fn parser_reads_soldier_award_icons() {
        let sample = r#"local _={}
_[4]={icon=7,called=1}
_[3]={icon=12}
_[2]={_[4],_[3]}
_[5]={id=1,name="A",awards=_[2]}
_[1]={_[5]}
return {deployedTroops=_[1]}"#;
        let summary = SaveSummary::from_ser_str("saveMNB3.lua", sample).unwrap();
        assert_eq!(summary.deployed_squad[0].award_icons, vec![7, 12]);
        assert_eq!(summary.deployed_squad[0].awards_count, 2);
    }

    /// Distinct values per mode, so a field reading another mode's variable shows up.
    #[test]
    fn parser_mode_fields_match_game_variables() {
        let sample = r#"return {rank=4,experience=1111,kills=222,casualty=33,dayz=9,missionCounter=5,mnb2Rank=2,mnb2Experience=4444,mnb2TotalKills=555,mnb2TotalCasualty=66,maxWave=77,currentWave=8,trenchModeRank=3,trenchModeXP=999}"#;
        let s = SaveSummary::from_ser_str("saveMNB3.lua", sample).unwrap();
        // Campaign (gameType 0): rank, experience, kills, casualty, dayz.
        assert_eq!(s.campaign_rank, 4);
        assert_eq!(s.campaign_experience, 1111);
        assert_eq!(s.campaign_kills, 222);
        assert_eq!(s.campaign_casualties, 33);
        assert_eq!(s.day, 9);
        assert_eq!(s.campaign_rank_title, "Captain");
        // Classic (mnb2, gameType 1): mnb2Rank, mnb2Experience, mnb2Total*, waves.
        assert_eq!(s.classic_rank, 2);
        assert_eq!(s.classic_experience, 4444);
        assert_eq!(s.classic_total_kills, 555);
        assert_eq!(s.classic_total_casualties, 66);
        assert_eq!(s.classic_max_wave, 77);
        assert_eq!(s.classic_current_wave, 8);
        assert_eq!(s.rank_title, classic_rank_title(2));
        // Trench.
        assert_eq!(s.trench_rank, 3);
        assert_eq!(s.trench_xp, 999);
    }

    const CLEAN_SAVE: &str = "local _={}\n_[2]={\"option_a\",\"option_b\"}\n_[1]={gcore=_[2]}\n_[3]={x=1}\nreturn {rank=4,option_a=1,[\"odd key\"]=\"v,}\",option_b=_[3],dayz=9,_qm_meta={profile_id=\"p\",fields=_[1]},7,last=2}";

    #[test]
    fn the_stamp_lists_the_fields_each_mod_owns() {
        let summary = SaveSummary::from_ser_str("saveMNB3.lua", CLEAN_SAVE).unwrap();
        let meta = summary.qm_meta.unwrap();
        assert_eq!(meta.profile_id.as_deref(), Some("p"));
        assert_eq!(meta.fields["gcore"], ["option_a", "option_b"]);
        // A stamp from before Phase E has no fields.
        let old = SaveSummary::from_ser_str("s", "return {_qm_meta={profile_id=\"p\"}}").unwrap();
        assert!(old.qm_meta.unwrap().fields.is_empty());
    }

    #[test]
    fn root_entry_spans_cover_every_root_entry() {
        let spans = SerDocument::root_entry_spans(CLEAN_SAVE).unwrap();
        let keys: Vec<_> = spans.iter().map(|e| e.key.as_deref()).collect();
        assert_eq!(
            keys,
            [
                Some("rank"),
                Some("option_a"),
                Some("odd key"),
                Some("option_b"),
                Some("dayz"),
                Some("_qm_meta"),
                None,
                Some("last")
            ]
        );
        let text = |i: usize| &CLEAN_SAVE[spans[i].entry.clone()];
        assert_eq!(text(0), "rank=4");
        assert_eq!(
            text(2),
            "[\"odd key\"]=\"v,}\"",
            "a separator inside a string is not one"
        );
        assert_eq!(text(3), "option_b=_[3]");
        assert_eq!(text(5), "_qm_meta={profile_id=\"p\",fields=_[1]}");
        assert_eq!(text(6), "7");
        assert_eq!(text(7), "last=2");

        // The memo form: the root is the `_[0]` line.
        let memo = "local _={}\n_[1]={}\n_[0]={a=1,b=_[1]}\n_[1].up=_[0]\nreturn _[0]";
        let spans = SerDocument::root_entry_spans(memo).unwrap();
        assert_eq!(spans.len(), 2);
        assert_eq!(&memo[spans[1].entry.clone()], "b=_[1]");
        assert!(SerDocument::root_entry_spans("local _={}\n").is_err());
    }

    #[test]
    fn removing_root_keys_keeps_every_other_byte() {
        let (text, removed) =
            SerDocument::remove_root_keys(CLEAN_SAVE, &["option_a", "option_b", "absent"]).unwrap();
        assert_eq!(removed, ["option_a", "option_b"]);
        assert_eq!(
            text,
            CLEAN_SAVE
                .replace("option_a=1,", "")
                .replace("option_b=_[3],", "")
        );
        let doc = SerDocument::parse(&text).unwrap();
        assert!(!doc.root.map.contains_key("option_a") && !doc.root.map.contains_key("option_b"));
        assert_eq!(doc.root.get_i64("rank", &doc.memo), Some(4));
        assert_eq!(doc.root.get_i64("last", &doc.memo), Some(2));
        assert_eq!(doc.root.array.len(), 1);

        // The last entry goes with the separator before it; so do entries removed next to it.
        let (text, _) = SerDocument::remove_root_keys("return {a=1,b=2,c=3}", &["c"]).unwrap();
        assert_eq!(text, "return {a=1,b=2}");
        let (text, _) = SerDocument::remove_root_keys("return {a=1,b=2,c=3}", &["b", "c"]).unwrap();
        assert_eq!(text, "return {a=1}");
        let (text, _) = SerDocument::remove_root_keys("return {a=1,b=2,c=3}", &["a", "c"]).unwrap();
        assert_eq!(text, "return {b=2}");
        let (text, removed) =
            SerDocument::remove_root_keys("return {a=1,b=2,c=3}", &["a", "b", "c"]).unwrap();
        assert_eq!((text.as_str(), removed.len()), ("return {}", 3));
        let (text, _) =
            SerDocument::remove_root_keys("return { a = 1 , b = 2 , }", &["b"]).unwrap();
        assert_eq!(text, "return { a = 1 , }");
        // Nothing to remove: the text is unchanged.
        let (text, removed) = SerDocument::remove_root_keys(CLEAN_SAVE, &["nope"]).unwrap();
        assert!(removed.is_empty());
        assert_eq!(text, CLEAN_SAVE);
        // Only root entries: a nested key of the same name stays.
        let (text, removed) =
            SerDocument::remove_root_keys("return {a={b=1},b=2}", &["b"]).unwrap();
        assert_eq!((text.as_str(), removed.len()), ("return {a={b=1}}", 1));
    }
}
