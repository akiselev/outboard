use crate::Manifest;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Mutex,
    time::UNIX_EPOCH,
};
const FORMAT: u32 = 1;
#[derive(Debug, Clone)]
pub enum CacheMode {
    Disabled,
    Memory,
    Disk(PathBuf),
    DefaultDisk,
}
impl Default for CacheMode {
    fn default() -> Self {
        Self::DefaultDisk
    }
}
#[derive(Debug)]
pub struct ManifestCache {
    enabled: bool,
    path: Option<PathBuf>,
    entries: Mutex<BTreeMap<String, CachedEntry>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedEntry {
    fingerprint: Fingerprint,
    manifest: Manifest,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Fingerprint {
    len: u64,
    modified_nanos: u128,
}
#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    format_version: u32,
    entries: BTreeMap<String, CachedEntry>,
}
impl ManifestCache {
    pub fn new(mode: CacheMode) -> Self {
        match mode {
            CacheMode::Disabled => Self {
                enabled: false,
                path: None,
                entries: Mutex::new(BTreeMap::new()),
            },
            CacheMode::Memory => Self {
                enabled: true,
                path: None,
                entries: Mutex::new(BTreeMap::new()),
            },
            CacheMode::Disk(p) => Self::load(p),
            CacheMode::DefaultDisk => default_path().map_or_else(
                || Self {
                    enabled: true,
                    path: None,
                    entries: Mutex::new(BTreeMap::new()),
                },
                Self::load,
            ),
        }
    }
    fn load(path: PathBuf) -> Self {
        let entries = fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<CacheFile>(&b).ok())
            .filter(|f| f.format_version == FORMAT)
            .map_or_else(BTreeMap::new, |f| f.entries);
        Self {
            enabled: true,
            path: Some(path),
            entries: Mutex::new(entries),
        }
    }
    pub fn get(&self, path: &Path) -> Option<Manifest> {
        if !self.enabled {
            return None;
        }
        let fp = fingerprint(path).ok()?;
        self.entries
            .lock()
            .ok()?
            .get(&key(path))
            .filter(|e| e.fingerprint == fp)
            .map(|e| e.manifest.clone())
    }
    pub fn insert(&self, path: &Path, manifest: Manifest) {
        if !self.enabled {
            return;
        }
        let Ok(fp) = fingerprint(path) else { return };
        if let Ok(mut g) = self.entries.lock() {
            g.insert(
                key(path),
                CachedEntry {
                    fingerprint: fp,
                    manifest,
                },
            );
        }
        let _ = self.persist();
    }
    pub fn invalidate(&self, path: &Path) {
        if !self.enabled {
            return;
        }
        if let Ok(mut g) = self.entries.lock() {
            g.remove(&key(path));
        }
        let _ = self.persist();
    }
    pub fn clear(&self) {
        if !self.enabled {
            return;
        }
        if let Ok(mut g) = self.entries.lock() {
            g.clear();
        }
        let _ = self.persist();
    }
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    fn persist(&self) -> Result<(), std::io::Error> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let entries = self.entries.lock().map(|g| g.clone()).unwrap_or_default();
        let bytes = serde_json::to_vec_pretty(&CacheFile {
            format_version: FORMAT,
            entries,
        })
        .expect("cache serializes");
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, bytes)?;
        match fs::rename(&tmp, path) {
            Ok(()) => Ok(()),
            Err(_) if path.exists() => {
                fs::remove_file(path)?;
                fs::rename(&tmp, path)
            }
            Err(e) => Err(e),
        }
    }
}
impl Default for ManifestCache {
    fn default() -> Self {
        Self::new(CacheMode::default())
    }
}
fn fingerprint(path: &Path) -> Result<Fingerprint, std::io::Error> {
    let m = fs::metadata(path)?;
    let modified_nanos = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    Ok(Fingerprint {
        len: m.len(),
        modified_nanos,
    })
}
fn key(path: &Path) -> String {
    let p = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut h = std::collections::hash_map::DefaultHasher::new();
    hash_path(&p, &mut h);
    format!("{:016x}", h.finish())
}
#[cfg(unix)]
fn hash_path(p: &Path, h: &mut impl Hasher) {
    use std::os::unix::ffi::OsStrExt;
    p.as_os_str().as_bytes().hash(h)
}
#[cfg(windows)]
fn hash_path(p: &Path, h: &mut impl Hasher) {
    use std::os::windows::ffi::OsStrExt;
    p.as_os_str().encode_wide().collect::<Vec<_>>().hash(h)
}
#[cfg(not(any(unix, windows)))]
fn hash_path(p: &Path, h: &mut impl Hasher) {
    p.to_string_lossy().hash(h)
}
fn default_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .map(|p| p.join("outboard/manifest-cache.json"))
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|p| p.join(".cache/outboard/manifest-cache.json"))
        })
}
