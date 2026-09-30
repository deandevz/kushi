//! Bridge from the Celestial mod manager: skins installed through Celestial
//! ("Open in Celestial" on divineskins.gg) are picked up from its local library
//! and packed into Zushi customs. Only reads files Celestial already downloaded.

use crate::state::CustomEntry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Manager};
use zip::write::SimpleFileOptions;

const CELESTIAL_STORAGE: &str = "Library/Application Support/com.divineskins.celestial/storage";
const STATE_FILE: &str = "celestial_sync.json";
/// Files touched this recently may still be written by Celestial.
const SETTLE_TIME: Duration = Duration::from_secs(3);

/// library.json mtime of the last complete sync, to skip work while nothing changed.
static LAST_SYNCED_MTIME: Mutex<Option<SystemTime>> = Mutex::new(None);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CelestialLibrary {
    #[serde(default)]
    mods: Vec<CelestialMod>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CelestialMod {
    id: String,
    name: String,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    divine_skin_id: Option<u64>,
    #[serde(default)]
    divine_version_id: Option<u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SyncState {
    /// Celestial mod id -> what we imported for it. Kept after the user removes
    /// the custom in Zushi, so a deleted skin is not re-imported.
    seen: HashMap<String, SeenMod>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SeenMod {
    version: String,
    file_name: String,
}

#[derive(Debug, Serialize)]
pub struct CelestialSyncResult {
    pub available: bool,
    /// True on the very first sync, when the whole existing library comes in.
    pub first_run: bool,
    pub imported: Vec<CustomEntry>,
}

fn flatten<T>(res: Result<Result<T, String>, impl std::fmt::Display>) -> Result<T, String> {
    match res {
        Ok(inner) => inner,
        Err(e) => Err(e.to_string()),
    }
}

fn storage_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = Path::new(&home).join(CELESTIAL_STORAGE);
    dir.join("library.json").is_file().then_some(dir)
}

fn load_state(path: &Path) -> Option<SyncState> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn save_state(path: &Path, state: &SyncState) -> Result<(), String> {
    let json = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| format!("Failed to save Celestial sync state: {e}"))
}

fn version_key(m: &CelestialMod) -> String {
    match m.divine_version_id {
        Some(v) => v.to_string(),
        None => m.version.clone().unwrap_or_default(),
    }
}

fn file_name_for(m: &CelestialMod) -> String {
    let safe: String = m
        .name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ' ' | '.' | '\''))
        .collect::<String>()
        .trim()
        .to_string();
    let safe = if safe.is_empty() { "Divine skin".to_string() } else { safe };
    let id = m.divine_skin_id.map(|i| i.to_string()).unwrap_or_else(|| m.id.clone());
    format!("{safe} (divine {id})")
}

fn newest_mtime(dir: &Path) -> Option<SystemTime> {
    let mut newest = fs::metadata(dir).ok()?.modified().ok();
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let t = if path.is_dir() {
            newest_mtime(&path)
        } else {
            entry.metadata().ok().and_then(|m| m.modified().ok())
        };
        if t > newest {
            newest = t;
        }
    }
    newest
}

/// Newer Divine uploads come as a single `.modpkg` instead of META/ + WAD/.
fn find_modpkg(dir: &Path) -> Option<PathBuf> {
    fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("modpkg"))
    })
}

/// Installed mod folder complete and no longer being written.
fn is_ready(dir: &Path) -> bool {
    let wad = dir.join("WAD");
    let has_wad = fs::read_dir(&wad).map(|mut d| d.next().is_some()).unwrap_or(false);
    if !has_wad && find_modpkg(dir).is_none() {
        return false;
    }
    match newest_mtime(dir).and_then(|t| t.elapsed().ok()) {
        Some(age) => age >= SETTLE_TIME,
        None => false,
    }
}

fn add_dir(
    zip: &mut zip::ZipWriter<File>,
    src: &Path,
    prefix: &str,
    opts: SimpleFileOptions,
) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(src).map_err(|e| e.to_string())?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let zip_path = format!("{prefix}/{name}");
        if path.is_dir() {
            add_dir(zip, &path, &zip_path, opts)?;
        } else {
            zip.start_file(zip_path, opts).map_err(|e| e.to_string())?;
            let mut f = File::open(&path).map_err(|e| e.to_string())?;
            std::io::copy(&mut f, zip).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Celestial keeps mods unpacked (META/ + WAD/ + thumbnail.webp). Pack them as a
/// regular .fantome, with the library metadata and thumbnail folded into META/.
fn pack(m: &CelestialMod, src: &Path, dest: &Path) -> Result<(), String> {
    let mut info: serde_json::Map<String, serde_json::Value> =
        fs::read_to_string(src.join("META/info.json"))
            .ok()
            .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_default();
    info.insert("Name".into(), m.name.clone().into());
    for (key, value) in [
        ("Author", &m.author),
        ("Version", &m.version),
        ("Description", &m.description),
    ] {
        if let Some(v) = value.as_ref().filter(|v| !v.is_empty()) {
            info.insert(key.into(), v.clone().into());
        }
    }

    let tmp = dest.with_extension("fantome.part");
    let file = File::create(&tmp).map_err(|e| format!("Failed to create mod file: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let deflate = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    // WAD contents are already compressed; storing them keeps packing fast.
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    let write = || -> Result<(), String> {
        zip.start_file("META/info.json", deflate).map_err(|e| e.to_string())?;
        let json = serde_json::to_vec_pretty(&info).map_err(|e| e.to_string())?;
        zip.write_all(&json).map_err(|e| e.to_string())?;

        let thumb = [
            ("thumbnail.webp", "META/image.webp"),
            ("META/image.png", "META/image.png"),
            ("META/thumbnail.webp", "META/image.webp"),
        ]
        .into_iter()
        .find(|(from, _)| src.join(from).is_file());
        if let Some((from, to)) = thumb {
            let mut bytes = Vec::new();
            File::open(src.join(from))
                .and_then(|mut f| f.read_to_end(&mut bytes))
                .map_err(|e| e.to_string())?;
            zip.start_file(to, stored).map_err(|e| e.to_string())?;
            zip.write_all(&bytes).map_err(|e| e.to_string())?;
        }

        match find_modpkg(src) {
            // The base layer unpacks as {wad}/{path}: raw WAD folders, which the
            // patcher builds into WADs like any other fantome.
            Some(modpkg) if !src.join("WAD").is_dir() => {
                let unpacked = tempdir(dest)?;
                let result = extract_modpkg(&modpkg, &unpacked)
                    .and_then(|_| add_dir(&mut zip, &unpacked, "WAD", stored));
                fs::remove_dir_all(&unpacked).ok();
                result?;
            }
            _ => add_dir(&mut zip, &src.join("WAD"), "WAD", stored)?,
        }
        zip.finish().map_err(|e| e.to_string())?;
        Ok(())
    };

    let result = write().and_then(|_| fs::rename(&tmp, dest).map_err(|e| e.to_string()));
    if result.is_err() {
        fs::remove_file(&tmp).ok();
    }
    result
}

fn tempdir(near: &Path) -> Result<PathBuf, String> {
    let dir = near.with_extension("unpack");
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn extract_modpkg(path: &Path, out: &Path) -> Result<(), String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut modpkg = ltk_modpkg::Modpkg::mount_from_reader(std::io::BufReader::new(file))
        .map_err(|e| format!("Invalid .modpkg: {e}"))?;
    let staging = out.join(".layers");
    ltk_modpkg::ModpkgExtractor::new(&mut modpkg)
        .extract_layer("base", &staging)
        .map_err(|e| format!("Failed to unpack .modpkg: {e}"))?;
    let base = staging.join("base");
    if !base.is_dir() {
        return Err(".modpkg has no base layer".into());
    }
    for entry in fs::read_dir(&base).map_err(|e| e.to_string())?.flatten() {
        fs::rename(entry.path(), out.join(entry.file_name())).map_err(|e| e.to_string())?;
    }
    fs::remove_dir_all(&staging).ok();
    Ok(())
}

#[tauri::command]
pub async fn sync_celestial(app: AppHandle) -> Result<CelestialSyncResult, String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    flatten(
        tauri::async_runtime::spawn_blocking(move || {
            let Some(storage) = storage_dir() else {
                return Ok(CelestialSyncResult { available: false, first_run: false, imported: vec![] });
            };
            let library_path = storage.join("library.json");
            let mtime = fs::metadata(&library_path).and_then(|m| m.modified()).ok();

            let mut last = LAST_SYNCED_MTIME.lock().map_err(|e| e.to_string())?;
            if mtime.is_some() && *last == mtime {
                return Ok(CelestialSyncResult { available: true, first_run: false, imported: vec![] });
            }

            let raw = fs::read_to_string(&library_path).map_err(|e| e.to_string())?;
            let library: CelestialLibrary = serde_json::from_str(&raw)
                .map_err(|e| format!("Could not read Celestial library: {e}"))?;

            let customs = data_dir.join("customs");
            fs::create_dir_all(&customs).map_err(|e| e.to_string())?;
            let state_path = data_dir.join(STATE_FILE);
            let loaded = load_state(&state_path);
            let first_run = loaded.is_none();
            let mut state = loaded.unwrap_or_default();

            let mut imported = Vec::new();
            let mut pending = false;

            // Only skins downloaded from Divine; mods imported into Celestial by
            // hand already exist as files the user can import directly.
            for m in library.mods.iter().filter(|m| m.source.as_deref() == Some("Divine")) {
                let version = version_key(m);
                if state.seen.get(&m.id).is_some_and(|s| s.version == version) {
                    continue;
                }
                let src = storage.join("installed").join(&m.id);
                if !is_ready(&src) {
                    pending = true;
                    continue;
                }

                let file_name = file_name_for(m);
                let dest = customs.join(format!("{file_name}.fantome"));
                if let Err(e) = pack(m, &src, &dest) {
                    eprintln!("[zushi] Celestial import failed for {}: {e}", m.name);
                    continue;
                }
                // A new version replaces the file imported for the previous one.
                if let Some(old) = state.seen.get(&m.id) {
                    if old.file_name != file_name {
                        fs::remove_file(customs.join(format!("{}.fantome", old.file_name))).ok();
                    }
                }
                state.seen.insert(m.id.clone(), SeenMod { version, file_name: file_name.clone() });
                imported.push(CustomEntry {
                    name: file_name,
                    file_path: dest.to_string_lossy().into_owned(),
                });
            }

            save_state(&state_path, &state)?;
            // Retry on the next poll while a download is still landing.
            *last = if pending { None } else { mtime };

            Ok(CelestialSyncResult { available: true, first_run, imported })
        })
        .await,
    )
}
