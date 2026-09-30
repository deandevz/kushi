use base64::Engine;
use serde::Serialize;
use std::fs::File;
use std::io::Read;
use std::path::Path;

const IMAGE_EXTS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];
const PREFERRED_STEMS: [&str; 8] = [
    "image", "thumbnail", "thumb", "preview", "cover", "icon", "splash", "banner",
];
const MAX_IMAGE_BYTES: u64 = 15 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct ModInfo {
    pub name: Option<String>,
    pub author: Option<String>,
    pub version: Option<String>,
    /// WAD names without extension or locale, e.g. "Shaco", "Fizz", "Map11".
    pub wads: Vec<String>,
    pub has_image: bool,
}

fn flatten<T>(res: Result<Result<T, String>, impl std::fmt::Display>) -> Result<T, String> {
    match res {
        Ok(inner) => inner,
        Err(e) => Err(e.to_string()),
    }
}

fn open_archive(path: &str) -> Result<zip::ZipArchive<File>, String> {
    let file = File::open(path).map_err(|e| format!("Failed to open mod: {e}"))?;
    zip::ZipArchive::new(file).map_err(|e| format!("Invalid mod archive: {e}"))
}

/// Mods may sit under an extra top folder ("ModName/WAD/..."), so look for the
/// WAD component anywhere in the path. Handles packed ("X.wad.client") and
/// raw-folder ("X.wad.client/data/...") WADs alike.
fn wad_name(entry: &str) -> Option<String> {
    let parts: Vec<&str> = entry.split('/').collect();
    let idx = parts.iter().position(|p| p.eq_ignore_ascii_case("WAD"))?;
    let wad = parts.get(idx + 1).filter(|w| !w.is_empty())?;
    let lower = wad.to_lowercase();
    if !(lower.ends_with(".wad.client") || lower.ends_with(".wad") || lower.ends_with(".wad.mobile")) {
        return None;
    }
    // "Fizz.en_US.wad.client" -> "Fizz"
    wad.split('.').next().map(|s| s.to_string())
}

fn in_wad(entry: &str) -> bool {
    entry.split('/').any(|p| p.eq_ignore_ascii_case("WAD") || p.eq_ignore_ascii_case("RAW"))
}

fn image_ext(entry: &str) -> Option<String> {
    let ext = Path::new(entry).extension()?.to_str()?.to_lowercase();
    IMAGE_EXTS.contains(&ext.as_str()).then_some(ext)
}

/// Pick the best preview image in the archive: prefer files in META/ and with
/// names like image/thumbnail/preview, then the biggest one.
fn best_image(archive: &mut zip::ZipArchive<File>) -> Option<(usize, String)> {
    let mut best: Option<(i64, u64, usize, String)> = None;
    for i in 0..archive.len() {
        let Ok(file) = archive.by_index_raw(i) else { continue };
        let name = file.name().to_string();
        if file.is_dir() || in_wad(&name) || file.size() == 0 || file.size() > MAX_IMAGE_BYTES {
            continue;
        }
        let Some(ext) = image_ext(&name) else { continue };
        if name.split('/').any(|p| p.starts_with('.') || p == "__MACOSX") {
            continue;
        }
        let stem = Path::new(&name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        let mut score = 0i64;
        if name.split('/').any(|p| p.eq_ignore_ascii_case("META")) {
            score += 10;
        }
        if PREFERRED_STEMS.iter().any(|p| stem.contains(p)) {
            score += 5;
        }
        let candidate = (score, file.size(), i, ext);
        if best.as_ref().map_or(true, |b| (candidate.0, candidate.1) > (b.0, b.1)) {
            best = Some(candidate);
        }
    }
    best.map(|(_, _, i, ext)| (i, ext))
}

fn read_info_json(archive: &mut zip::ZipArchive<File>) -> Option<serde_json::Value> {
    let idx = (0..archive.len()).find(|&i| {
        archive
            .by_index_raw(i)
            .map(|f| f.name().to_lowercase().ends_with("meta/info.json"))
            .unwrap_or(false)
    })?;
    let mut buf = String::new();
    archive.by_index(idx).ok()?.read_to_string(&mut buf).ok()?;
    serde_json::from_str(buf.trim_start_matches('\u{feff}')).ok()
}

fn json_str(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(|x| x.as_str()))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[tauri::command]
pub async fn read_mod_info(path: String) -> Result<ModInfo, String> {
    flatten(
        tauri::async_runtime::spawn_blocking(move || {
            let mut archive = open_archive(&path)?;

            let mut wads: Vec<String> = Vec::new();
            for i in 0..archive.len() {
                if let Ok(f) = archive.by_index_raw(i) {
                    if let Some(w) = wad_name(f.name()) {
                        if !wads.iter().any(|x| x.eq_ignore_ascii_case(&w)) {
                            wads.push(w);
                        }
                    }
                }
            }

            let info = read_info_json(&mut archive);
            let has_image = best_image(&mut archive).is_some();

            Ok(ModInfo {
                name: info.as_ref().and_then(|v| json_str(v, &["Name", "name"])),
                author: info.as_ref().and_then(|v| json_str(v, &["Author", "author"])),
                version: info.as_ref().and_then(|v| json_str(v, &["Version", "version"])),
                wads,
                has_image,
            })
        })
        .await,
    )
}

/// Returns the mod's preview image as a data URL, or None if it has none.
#[tauri::command]
pub async fn read_mod_image(path: String) -> Result<Option<String>, String> {
    flatten(
        tauri::async_runtime::spawn_blocking(move || {
            let mut archive = open_archive(&path)?;
            let Some((idx, ext)) = best_image(&mut archive) else {
                return Ok(None);
            };
            let mut bytes = Vec::new();
            archive
                .by_index(idx)
                .map_err(|e| e.to_string())?
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            let mime = match ext.as_str() {
                "jpg" | "jpeg" => "image/jpeg",
                "webp" => "image/webp",
                "gif" => "image/gif",
                _ => "image/png",
            };
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            Ok(Some(format!("data:{mime};base64,{b64}")))
        })
        .await,
    )
}
