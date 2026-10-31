use super::{OnFailurePolicy, Priority, Target};
use crate::dump::{ByteDebugEntry, ByteRegion, DebugPatchType, PatchSource};
use crop::Rope;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CopyPosition {
    Prepend,
    Append,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CopyPatch {
    pub position: CopyPosition,
    pub target: Target,
    pub sources: Option<Vec<PathBuf>>,

    pub payload: Option<String>,

    pub id: Option<String>,
    pub name: Option<String>,
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

    // Buffer contents read at load time. We do this to support zip-based mods that we can't arbitrarily read from.
    #[serde(skip)]
    pub contents: Vec<String>,
}

impl CopyPatch {
    /// Apply a copy patch onto the provided buffer and name.
    pub fn apply(&self, target: &str, rope: &mut Rope, path: &Path) -> Option<ByteDebugEntry> {
        if !self.target.can_apply(target) {
            return None;
        }

        let mut before = 0;
        let mut after = 0;

        // Combine contents and payload into a single iterator. Contents, then payload (if defined).
        let payloads = self
            .contents
            .iter()
            .map(|s| s.as_str())
            .chain(self.payload.as_deref());

        for content in payloads {
            match self.position {
                CopyPosition::Prepend => {
                    before += content.len() + 1;
                    rope.insert(0, "\n");
                    rope.insert(0, content);
                }
                CopyPosition::Append => {
                    after += content.len() + 1;
                    rope.insert(rope.byte_len(), "\n");
                    rope.insert(rope.byte_len(), content);
                }
            }
        }

        let mut byte_regions = Vec::new();
        if before != 0 {
            byte_regions.push(ByteRegion {
                start: 0,
                end: before,
                delta: before as isize,
            });
        }
        if after != 0 {
            let len = rope.byte_len();
            byte_regions.push(ByteRegion {
                start: len - after,
                end: len,
                delta: after as isize,
            });
        }
        Some(ByteDebugEntry {
            patch_source: PatchSource {
                file: path.display().to_string(),
                mod_id: self.mod_id.clone(),
                patch_id: self.id.clone(),
                patch_name: self.name.clone(),
                pattern: None,
                patch_type: DebugPatchType::Copy,
            },
            regions: byte_regions,
            matched_count: 1,
            expected_count: Some(1),
            warnings: None,
        })
    }
}
