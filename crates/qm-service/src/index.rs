//! The mod index (spec 8.5): one conditional request for every published mod. The last good
//! copy is kept in `cache/mod-index.json`, so an unreachable site (an outage, or the account
//! suspended) leaves the launcher working from it.
use crate::service::Service;
use qm_manager::community::fetch_conditional;
use qm_schema::IndexFile;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(crate) const INDEX_CACHE_FILE: &str = "mod-index.json";
const INDEX_META_FILE: &str = "mod-index.meta.json";
const USER_AGENT: &str = "QuarterMaster-Launcher/1.0 (Mud and Blood Mod Manager)";
/// The index is a few hundred KB; anything near this is not the index.
const MAX_INDEX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct IndexMeta {
    etag: Option<String>,
    /// When the stored copy was last confirmed current (200 or 304).
    fetched_at: Option<String>,
}

/// The index as the launcher sees it now.
#[derive(Debug, Clone, Default)]
pub(crate) struct IndexSnapshot {
    pub file: IndexFile,
    /// The site could not be reached (or sent something invalid) on this attempt.
    pub offline: bool,
    /// When the copy in use was last confirmed current; `None` when there is no copy.
    pub copy_from: Option<String>,
}

impl Service {
    /// The index: with `network`, one conditional request (304 keeps the stored copy); without,
    /// the stored copy only.
    pub(crate) fn index_load(&self, network: bool) -> IndexSnapshot {
        let dir = self.paths().cache_dir.clone();
        let body_path = dir.join(INDEX_CACHE_FILE);
        let meta_path = dir.join(INDEX_META_FILE);
        let stored = std::fs::read(&body_path)
            .ok()
            .and_then(|b| IndexFile::parse(&b).ok());
        let mut meta: IndexMeta = read_json(&meta_path).unwrap_or_default();
        let offline = |stored: Option<IndexFile>, meta: &IndexMeta| IndexSnapshot {
            copy_from: stored.as_ref().and(meta.fetched_at.clone()),
            file: stored.unwrap_or_default(),
            offline: true,
        };
        if !network {
            return IndexSnapshot {
                copy_from: stored.as_ref().and(meta.fetched_at.clone()),
                file: stored.unwrap_or_default(),
                offline: false,
            };
        }
        let mut headers = vec![("User-Agent", USER_AGENT.to_string())];
        if let Some(etag) = meta.etag.as_ref().filter(|_| stored.is_some()) {
            headers.push(("If-None-Match", etag.clone()));
        }
        let net = self.net();
        let resp = match fetch_conditional(
            &net.index_url,
            &headers,
            net.allow_index.as_ref(),
            MAX_INDEX_BYTES,
        ) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("mod index unreachable: {e}");
                return offline(stored, &meta);
            }
        };
        let now = jiff::Zoned::now().to_string();
        match resp.status {
            304 if stored.is_some() => {
                meta.fetched_at = Some(now.clone());
                write_json(&meta_path, &meta);
                IndexSnapshot {
                    file: stored.unwrap_or_default(),
                    offline: false,
                    copy_from: Some(now),
                }
            }
            200 => match IndexFile::parse(&resp.body) {
                Ok(file) => {
                    if let Err(e) = write_atomic(&body_path, &resp.body) {
                        tracing::warn!("could not store the mod index: {e}");
                    }
                    write_json(
                        &meta_path,
                        &IndexMeta {
                            etag: resp.etag,
                            fetched_at: Some(now.clone()),
                        },
                    );
                    IndexSnapshot {
                        file,
                        offline: false,
                        copy_from: Some(now),
                    }
                }
                Err(e) => {
                    // Never replace a good copy with a broken one.
                    tracing::warn!("mod index rejected: {e}");
                    offline(stored, &meta)
                }
            },
            s => {
                tracing::warn!("mod index answered HTTP {s}");
                offline(stored, &meta)
            }
        }
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn write_json(path: &Path, value: &impl Serialize) {
    if let Ok(bytes) = serde_json::to_vec_pretty(value) {
        if let Err(e) = write_atomic(path, &bytes) {
            tracing::warn!("could not write {}: {e}", path.display());
        }
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp: PathBuf = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}
