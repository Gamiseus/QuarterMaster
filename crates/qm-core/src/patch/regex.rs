use std::path::Path;

use regex_cursor::engines::meta::Regex;
use regex_cursor::regex_automata::util::interpolate;
use regex_cursor::regex_automata::util::syntax;
use regex_cursor::Input;

use crop::Rope;
use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::chunk_vec_cursor::IntoCursor;
use crate::dump::{ByteDebugEntry, ByteRegion, DebugPatchType, PatchSource};

use super::{InsertPosition, OnFailurePolicy, Priority, Target};

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RegexPatch {
    pub target: Target,

    // The Regex pattern that will be used to both match and create capture groups.
    pub pattern: String,

    // The position to insert the payload relative to the match/capture group.
    pub position: InsertPosition,

    // The target root capture group. The insert position is relative to this group.
    // Defaults to $0 if not set (the entire match).
    pub root_capture: Option<String>,

    // The payload that will be inserted. Regex capture groups can be interpolated
    // by $index.
    pub payload: String,

    // A string or Regex capture to prepend onto the start of each LINE of the payload.
    // This value defaults to an empty string.
    #[serde(default)]
    pub line_prepend: String,

    // Apply patch at most `times` (or `expect`) times, warn if the number of matches differs.
    #[serde(default, alias = "expect")]
    pub times: Option<usize>,

    // If enabled, whitespace is ignored unless escaped
    #[serde(default)]
    pub verbose: bool,

    pub id: Option<String>,
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub requires: Vec<String>,
    pub priority: Option<Priority>,
    #[serde(default)]
    pub before: Vec<String>,
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default)]
    pub on_failure: OnFailurePolicy,

    #[serde(skip)]
    pub mod_id: Option<String>,

    #[serde(skip)]
    pub disabled: bool,
}

impl RegexPatch {
    pub fn debug_from_warning_string(&self, path: &Path, warning: String) -> ByteDebugEntry {
        log::warn!("{}", warning);
        ByteDebugEntry {
            patch_source: PatchSource {
                file: path.display().to_string(),
                mod_id: self.mod_id.clone(),
                patch_id: self.id.clone(),
                patch_name: self.name.clone(),
                pattern: Some(self.pattern.clone()),
                patch_type: DebugPatchType::Regex,
            },
            regions: Vec::new(),
            matched_count: 0,
            expected_count: self.times,
            warnings: Some(vec![warning]),
        }
    }

    pub fn apply(&self, target: &str, rope: &mut Rope, path: &Path) -> Option<ByteDebugEntry> {
        if !self.target.can_apply(target) {
            return None;
        }

        let input = Input::new(rope.into_cursor());
        let re = match Regex::builder()
            .syntax(
                syntax::Config::new()
                    .multi_line(true)
                    .crlf(true)
                    .ignore_whitespace(self.verbose),
            )
            .build(&self.pattern)
        {
            Ok(r) => r,
            Err(e) => {
                return Some(self.debug_from_warning_string(
                    path,
                    format!(
                        "Failed to compile Regex '{}' for regex patch from {}: {e:?}",
                        self.pattern,
                        path.display()
                    ),
                ));
            }
        };

        let mut captures = re.captures_iter(input).collect_vec();
        if captures.is_empty() {
            let warning = format!(
                "Regex '{}' on target '{target}' for regex patch from {} resulted in no matches",
                self.pattern.escape_debug(),
                path.display()
            );
            return Some(self.debug_from_warning_string(path, warning));
        }
        let raw_match_count = captures.len();
        let mut warnings = Vec::new();
        if let Some(times) = self.times {
            fn warn_regex_mismatch(
                pattern: &str,
                target: &str,
                found_matches: usize,
                wanted_matches: usize,
                path: &Path,
            ) -> String {
                let warn_msg: String = if pattern.lines().count() > 1 {
                    format!("Regex '''\n{pattern}''' on target '{target}' for regex patch from {} resulted in {found_matches} matches, wanted {wanted_matches}", path.display())
                } else {
                    format!("Regex '{pattern}' on target '{target}' for regex patch from {} resulted in {found_matches} matches, wanted {wanted_matches}", path.display())
                };
                for line in warn_msg.lines() {
                    log::warn!("{}", line)
                }
                warn_msg
            }
            if captures.len() < times {
                warnings.push(warn_regex_mismatch(
                    &self.pattern,
                    target,
                    captures.len(),
                    times,
                    path,
                ));
            }
            if captures.len() > times {
                warnings.push(warn_regex_mismatch(
                    &self.pattern,
                    target,
                    captures.len(),
                    times,
                    path,
                ));
                log::warn!("Ignoring excess matches");
                warnings.push("Ignoring excess matches".to_string());
                captures.truncate(times);
            }
        }

        // Running byte offset to keep byte references valid after rope mutations.
        let mut delta = 0_isize;

        // Collect byte regions during patching.
        let mut byte_regions: Vec<ByteRegion> = Vec::new();

        for groups in captures {
            // Get the entire captured span (index 0);
            let base = groups.get_group(0).unwrap();
            let base_start = (base.start as isize + delta) as usize;
            let base_end = (base.end as isize + delta) as usize;

            let base_str = rope.byte_slice(base_start..base_end).to_string();

            // Interpolate capture groups into self.line_prepend, if any capture groups exist within.
            let mut line_prepend = String::new();
            interpolate::string(
                &self.line_prepend,
                |index, dest| {
                    let span = groups.get_group(index).unwrap();
                    let start = (span.start as isize + delta) as usize;
                    let end = (span.end as isize + delta) as usize;

                    let rope_slice = rope.byte_slice(start..end);

                    dest.push_str(&rope_slice.to_string());
                },
                |name| {
                    let pid = groups.pattern().unwrap();
                    groups.group_info().to_index(pid, name)
                },
                &mut line_prepend,
            );

            // Cleanup and convert the specified root capture to a span.
            let target_group = {
                let group_name = self.root_capture.as_deref().unwrap_or("0").replace('$', "");

                if let Ok(idx) = group_name.parse::<usize>() {
                    groups.get_group(idx).unwrap_or_else(|| {
                        panic!("The capture group at index {idx} could not be found in '{base_str}' with the Regex '{}' for regex patch from {}", self.pattern, path.display())
                    })
                } else {
                    groups.get_group_by_name(&group_name).unwrap_or_else(|| {
                        panic!("The capture group with name '{group_name}' could not be found in '{base_str}' with the Regex '{}' for regex patch from {}", self.pattern, path.display())
                    })
                }
            };

            let target_start = (target_group.start as isize + delta) as usize;
            let target_end = (target_group.end as isize + delta) as usize;

            let new_payload = std::format!(
                "{}",
                self.payload
                    .split_inclusive('\n')
                    .format_with("", |x, f| f(&format_args!("{}{}", line_prepend, x)))
            );

            let mut payload = String::new();

            interpolate::string(
                &new_payload,
                |index, dest| {
                    let span = groups.get_group(index).unwrap();
                    let start = (span.start as isize + delta) as usize;
                    let end = (span.end as isize + delta) as usize;

                    let rope_slice = rope.byte_slice(start..end);

                    dest.push_str(&rope_slice.to_string());
                },
                |name| {
                    let pid = groups.pattern().unwrap();
                    groups.group_info().to_index(pid, name)
                },
                &mut payload,
            );

            if payload.starts_with(|x: char| x.is_ascii_alphanumeric() || x == '_') {
                let pre_pt = if let InsertPosition::After = self.position {
                    target_end
                } else {
                    target_start
                };
                if pre_pt > 0 {
                    let byte_on_left = rope.byte(pre_pt - 1);
                    if byte_on_left.is_ascii_alphanumeric() || byte_on_left == b'_' {
                        payload.insert(0, ' ');
                    }
                }
            }

            if payload.ends_with(|x: char| x.is_ascii_alphanumeric() || x == '_') {
                let post_pt = if let InsertPosition::Before = self.position {
                    target_start
                } else {
                    target_end
                };
                if post_pt < rope.byte_len() {
                    let byte_on_right = rope.byte(post_pt);
                    if byte_on_right.is_ascii_alphanumeric() || byte_on_right == b'_' {
                        payload.push(' ');
                    }
                }
            }

            let payload_bytes = payload.len();

            match self.position {
                InsertPosition::Before => {
                    rope.insert(target_start, &payload);
                    byte_regions.push(ByteRegion {
                        start: target_start,
                        end: target_start + payload_bytes,
                        delta: payload_bytes as isize,
                    });
                    delta += payload_bytes as isize;
                }
                InsertPosition::After => {
                    rope.insert(target_end, &payload);
                    byte_regions.push(ByteRegion {
                        start: target_end,
                        end: target_end + payload_bytes,
                        delta: payload_bytes as isize,
                    });
                    delta += payload_bytes as isize;
                }
                InsertPosition::At => {
                    let old_len = target_group.end - target_group.start;
                    rope.delete(target_start..target_end);
                    rope.insert(target_start, &payload);
                    byte_regions.push(ByteRegion {
                        start: target_start,
                        end: target_start + payload_bytes,
                        delta: payload_bytes as isize - old_len as isize,
                    });
                    delta -= old_len as isize;
                    delta += payload_bytes as isize;
                }
            }
        }

        Some(ByteDebugEntry {
            patch_source: PatchSource {
                file: path.display().to_string(),
                mod_id: self.mod_id.clone(),
                patch_id: self.id.clone(),
                patch_name: self.name.clone(),
                pattern: Some(self.pattern.clone()),
                patch_type: DebugPatchType::Regex,
            },
            regions: byte_regions,
            matched_count: raw_match_count,
            expected_count: self.times,
            warnings: if warnings.is_empty() {
                None
            } else {
                Some(warnings)
            },
        })
    }
}
