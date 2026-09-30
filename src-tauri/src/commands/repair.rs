//! Patch-compatibility repair for mods, run right before they are applied.
//!
//! League moved asset paths in .bin files from `string` to `file` (a 64-bit
//! path hash, WadChunkLink). Mods built before that still store strings, and
//! the game dies loading them ("FATAL ERROR. Missing data"). This learns, from
//! the installed game's own WADs, which fields are `file` now, and rewrites the
//! mod's strings in those fields. Mods are never edited in place: repaired
//! copies live in a cache keyed by the mod file and the game WADs it touches,
//! so a new patch re-runs the repair from the original.

use indexmap::IndexMap;
use ltk_hash::{BinHash, WadHash};
use ltk_meta::{
    property::{values, Kind},
    Bin, PropertyValueEnum as V,
};
use ltk_wad::{Wad, WadBuilder, WadChunkBuilder};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::UNIX_EPOCH;

// ---------------------------------------------------------------------------
// Field kinds learned from the game

/// Which fields the installed game stores as `file` and which as `string`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FieldKinds {
    file: HashSet<u32>,
    string: HashSet<u32>,
    /// (class, field) pairs, for fields whose type differs between classes.
    file_in: HashSet<(u32, u32)>,
    string_in: HashSet<(u32, u32)>,
}

fn leaf_kind(v: &V) -> Kind {
    match v {
        V::Optional(o) => o.item_kind(),
        V::Container(c) => c.item_kind(),
        V::UnorderedContainer(c) => c.0.item_kind(),
        other => other.kind(),
    }
}

impl FieldKinds {
    fn merge(&mut self, other: &FieldKinds) {
        self.file.extend(&other.file);
        self.string.extend(&other.string);
        self.file_in.extend(&other.file_in);
        self.string_in.extend(&other.string_in);
    }

    fn learn(&mut self, bin: &Bin) {
        for (_, obj) in bin.iter() {
            self.record_props(*obj.class_hash, &obj.properties);
        }
    }

    fn record_props(&mut self, class: u32, props: &IndexMap<BinHash, V>) {
        for (f, v) in props {
            match leaf_kind(v) {
                Kind::WadChunkLink => {
                    self.file.insert(**f);
                    self.file_in.insert((class, **f));
                }
                Kind::String => {
                    self.string.insert(**f);
                    self.string_in.insert((class, **f));
                }
                _ => {}
            }
            self.descend(v);
        }
    }

    fn descend(&mut self, v: &V) {
        match v {
            V::Struct(s) => self.record_props(*s.class_hash, &s.properties),
            V::Embedded(e) => self.record_props(*e.0.class_hash, &e.0.properties),
            V::Container(c) => c.items().iter().for_each(|i| self.descend(i)),
            V::UnorderedContainer(c) => c.0.items().iter().for_each(|i| self.descend(i)),
            V::Optional(o) => {
                if let Some(i) = o.value() {
                    self.descend(i)
                }
            }
            V::Map(m) => m.entries().iter().for_each(|(_, val)| self.descend(val)),
            _ => {}
        }
    }

    /// The class's own usage wins; a pair the game never shows falls back to the field alone.
    fn convertible(&self, class: u32, field: u32) -> bool {
        if self.file_in.contains(&(class, field)) {
            return true;
        }
        if self.string_in.contains(&(class, field)) {
            return false;
        }
        self.file.contains(&field) && !self.string.contains(&field)
    }

    fn is_empty(&self) -> bool {
        self.file.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Migration

fn link(s: &str) -> V {
    let hash = if s.is_empty() { WadHash::from(0u64) } else { WadHash::from(s) };
    V::WadChunkLink(values::WadChunkLink::new(hash))
}

fn take(v: &mut V) -> V {
    std::mem::replace(v, V::None(Default::default()))
}

fn to_links(items: Vec<V>) -> Vec<V> {
    items
        .into_iter()
        .map(|i| match i {
            V::String(s) => link(&s.value),
            x => x,
        })
        .collect()
}

/// Rewrites string asset paths the game now reads as `file`. Returns how many values changed.
fn migrate_bin(k: &FieldKinds, bin: &mut Bin) -> usize {
    bin.iter_mut()
        .map(|(_, obj)| {
            let class = *obj.class_hash;
            migrate_props(k, class, &mut obj.properties)
        })
        .sum()
}

fn migrate_props(k: &FieldKinds, class: u32, props: &mut IndexMap<BinHash, V>) -> usize {
    props.iter_mut().map(|(f, v)| migrate(k, class, **f, v)).sum()
}

fn migrate(k: &FieldKinds, class: u32, field: u32, v: &mut V) -> usize {
    let conv = k.convertible(class, field);
    match v {
        V::String(s) if conv => {
            *v = link(&s.value);
            1
        }
        V::Optional(o) if conv && o.item_kind() == Kind::String => {
            let V::Optional(o) = take(v) else { unreachable!() };
            let inner = o.into_inner().map(|i| match i {
                V::String(s) => link(&s.value),
                x => x,
            });
            *v = V::Optional(values::Optional::new(Kind::WadChunkLink, inner).expect("option of file"));
            1
        }
        V::Container(c) if conv && c.item_kind() == Kind::String => {
            let V::Container(c) = take(v) else { unreachable!() };
            let items = to_links(c.into_items());
            let n = items.len().max(1);
            *v = V::Container(values::Container::new(Kind::WadChunkLink, items).expect("list of file"));
            n
        }
        V::UnorderedContainer(c) if conv && c.0.item_kind() == Kind::String => {
            let V::UnorderedContainer(c) = take(v) else { unreachable!() };
            let items = to_links(c.0.into_items());
            let n = items.len().max(1);
            *v = V::UnorderedContainer(values::UnorderedContainer(
                values::Container::new(Kind::WadChunkLink, items).expect("list of file"),
            ));
            n
        }
        V::Struct(s) => {
            let c = *s.class_hash;
            migrate_props(k, c, &mut s.properties)
        }
        V::Embedded(e) => {
            let c = *e.0.class_hash;
            migrate_props(k, c, &mut e.0.properties)
        }
        V::Container(_) | V::UnorderedContainer(_) | V::Optional(_) | V::Map(_) => {
            migrate_nested(k, class, field, v)
        }
        _ => 0,
    }
}

/// Containers of structs: rebuild them with their items migrated.
fn migrate_nested(k: &FieldKinds, class: u32, field: u32, v: &mut V) -> usize {
    let mut n = 0;
    let mut each = |mut i: V| {
        n += migrate(k, class, field, &mut i);
        i
    };
    let rebuilt = match take(v) {
        V::Container(c) => {
            let kind = c.item_kind();
            let items = c.into_items().into_iter().map(&mut each).collect();
            V::Container(values::Container::new(kind, items).expect("same item kind"))
        }
        V::UnorderedContainer(c) => {
            let kind = c.0.item_kind();
            let items = c.0.into_items().into_iter().map(&mut each).collect();
            V::UnorderedContainer(values::UnorderedContainer(
                values::Container::new(kind, items).expect("same item kind"),
            ))
        }
        V::Optional(o) => {
            let kind = o.item_kind();
            V::Optional(values::Optional::new(kind, o.into_inner().map(&mut each)).expect("same item kind"))
        }
        V::Map(m) => {
            let (kk, vk) = (m.key_kind(), m.value_kind());
            let entries = m.into_entries().into_iter().map(|(key, val)| (key, each(val))).collect();
            V::Map(values::Map::new(kk, vk, entries).expect("same kinds"))
        }
        other => other,
    };
    *v = rebuilt;
    n
}

fn is_bin(data: &[u8]) -> bool {
    data.starts_with(b"PROP") || data.starts_with(b"PTCH")
}

/// Migrates one .bin; None when nothing needed changing.
fn repair_bin(k: &FieldKinds, data: &[u8]) -> Option<(Vec<u8>, usize)> {
    let mut bin = Bin::from_reader(&mut Cursor::new(data)).ok()?;
    let n = migrate_bin(k, &mut bin);
    if n == 0 {
        return None;
    }
    let mut out = Cursor::new(Vec::new());
    bin.to_writer(&mut out).ok()?;
    Some((out.into_inner(), n))
}

/// Rebuilds a packed WAD with its bins migrated; None when nothing needed changing.
fn repair_wad(k: &FieldKinds, bytes: Vec<u8>) -> Result<Option<(Vec<u8>, usize)>, String> {
    let mut wad = Wad::mount(Cursor::new(bytes)).map_err(|e| format!("Invalid WAD: {e}"))?;
    let chunks: Vec<_> = wad.chunks().iter().cloned().collect();
    let mut data: HashMap<WadHash, Vec<u8>> = HashMap::with_capacity(chunks.len());
    let mut converted = 0;
    for chunk in &chunks {
        let raw = wad.load_chunk_decompressed(chunk).map_err(|e| e.to_string())?;
        let bytes = match is_bin(&raw).then(|| repair_bin(k, &raw)).flatten() {
            Some((fixed, n)) => {
                converted += n;
                fixed
            }
            None => raw.into_vec(),
        };
        data.insert(chunk.path_hash(), bytes);
    }
    if converted == 0 {
        return Ok(None);
    }

    let builder = data
        .keys()
        .fold(WadBuilder::default(), |b, h| b.with_chunk(WadChunkBuilder::default().with_hash(*h)));
    let mut out = Cursor::new(Vec::new());
    builder
        .build_to_writer(&mut out, |hash, cursor| {
            cursor.write_all(&data[&hash])?;
            Ok(())
        })
        .map_err(|e| format!("Failed to rebuild WAD: {e}"))?;
    Ok(Some((out.into_inner(), converted)))
}

// ---------------------------------------------------------------------------
// Game index and caches

/// Lowercased WAD file name -> path, for everything under DATA/FINAL.
fn game_wads(game_dir: &str) -> Arc<HashMap<String, PathBuf>> {
    static INDEX: OnceLock<Mutex<HashMap<String, Arc<HashMap<String, PathBuf>>>>> = OnceLock::new();
    let mut cache = INDEX.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = cache.get(game_dir) {
        return found.clone();
    }
    let mut map = HashMap::new();
    let mut stack = vec![Path::new(game_dir).join("DATA").join("FINAL")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.to_lowercase().ends_with(".wad.client") {
                    map.insert(name.to_lowercase(), path);
                }
            }
        }
    }
    let map = Arc::new(map);
    cache.insert(game_dir.to_string(), map.clone());
    map
}

fn file_stamp(path: &Path) -> String {
    let meta = fs::metadata(path).ok();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{size}-{mtime}")
}

fn short_hash(s: &str) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Field kinds of one game WAD, cached in memory and on disk per WAD version.
fn kinds_for(game_wad: &Path, cache_dir: &Path) -> Result<Arc<FieldKinds>, String> {
    static MEMO: OnceLock<Mutex<HashMap<String, Arc<FieldKinds>>>> = OnceLock::new();
    let key = format!("{}@{}", game_wad.display(), file_stamp(game_wad));
    if let Some(k) = MEMO.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(k.clone());
    }

    let disk = cache_dir.join(format!("kinds-{}.json", short_hash(&key)));
    let kinds = match fs::read(&disk).ok().and_then(|b| serde_json::from_slice::<FieldKinds>(&b).ok()) {
        Some(k) => k,
        None => {
            let mut k = FieldKinds::default();
            let mut wad = Wad::mount(File::open(game_wad).map_err(|e| e.to_string())?)
                .map_err(|e| format!("Failed to read game WAD: {e}"))?;
            let chunks: Vec<_> = wad.chunks().iter().cloned().collect();
            for chunk in &chunks {
                let Ok(data) = wad.load_chunk_decompressed(chunk) else { continue };
                if is_bin(&data) {
                    if let Ok(bin) = Bin::from_reader(&mut Cursor::new(&data[..])) {
                        k.learn(&bin);
                    }
                }
            }
            if let Ok(json) = serde_json::to_vec(&k) {
                fs::write(&disk, json).ok();
            }
            k
        }
    };
    let kinds = Arc::new(kinds);
    MEMO.get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, kinds.clone());
    Ok(kinds)
}

// ---------------------------------------------------------------------------
// Mod archives

/// "WAD/Shaco.wad.client" -> ("shaco.wad.client", packed), and
/// "WAD/Shaco.wad.client/data/x.bin" -> ("shaco.wad.client", raw).
fn wad_of(entry: &str) -> Option<(String, bool)> {
    let parts: Vec<&str> = entry.split('/').filter(|p| !p.is_empty()).collect();
    let idx = parts.iter().position(|p| p.eq_ignore_ascii_case("WAD"))?;
    let wad = parts.get(idx + 1)?;
    if !wad.to_lowercase().ends_with(".wad.client") {
        return None;
    }
    Some((wad.to_lowercase(), parts.len() == idx + 2))
}

/// "Fizz.en_US.wad.client" -> "fizz.wad.client": localized WADs hold no bins, so
/// learn from the base WAD of the same name.
fn base_wad_name(name: &str) -> String {
    let stem = name.split('.').next().unwrap_or(name);
    format!("{stem}.wad.client")
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheRecord {
    converted: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Prepared {
    /// The file to hand to the patcher: the repaired copy or the original.
    pub path: String,
    /// Values rewritten for the current patch (0 = mod was already compatible).
    pub converted: usize,
}

/// Returns a copy of the mod repaired for the installed game, or the mod itself
/// when it needs nothing. Any failure falls back to the original file.
pub fn prepare(src: &str, game_dir: &str, cache_dir: &Path) -> Prepared {
    match prepare_inner(src, game_dir, cache_dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[zushi] repair skipped for {src}: {e}");
            Prepared { path: src.to_string(), converted: 0 }
        }
    }
}

fn prepare_inner(src: &str, game_dir: &str, cache_dir: &Path) -> Result<Prepared, String> {
    fs::create_dir_all(cache_dir).map_err(|e| e.to_string())?;
    let mut archive =
        zip::ZipArchive::new(File::open(src).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    // Which game WADs this mod touches decides the field table and the cache key.
    let wads = game_wads(game_dir);
    let mut touched: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        if let Some((name, _)) = archive.by_index_raw(i).ok().and_then(|f| wad_of(f.name())) {
            let base = base_wad_name(&name);
            if wads.contains_key(&base) && !touched.contains(&base) {
                touched.push(base);
            }
        }
    }
    if touched.is_empty() {
        return Ok(Prepared { path: src.to_string(), converted: 0 });
    }
    touched.sort();

    let stamp = touched
        .iter()
        .map(|w| format!("{w}:{}", file_stamp(&wads[w])))
        .collect::<Vec<_>>()
        .join(",");
    let key = short_hash(&format!("{src}|{}|{stamp}", file_stamp(Path::new(src))));
    let out_path = cache_dir.join(format!("{key}.fantome"));
    let record_path = cache_dir.join(format!("{key}.json"));
    if let Some(rec) = fs::read(&record_path).ok().and_then(|b| serde_json::from_slice::<CacheRecord>(&b).ok()) {
        if rec.converted == 0 {
            return Ok(Prepared { path: src.to_string(), converted: 0 });
        }
        if out_path.is_file() {
            return Ok(Prepared { path: out_path.to_string_lossy().into_owned(), converted: rec.converted });
        }
    }

    let mut kinds_by_wad: HashMap<String, Arc<FieldKinds>> = HashMap::new();
    for w in &touched {
        let k = kinds_for(&wads[w], cache_dir)?;
        if !k.is_empty() {
            kinds_by_wad.insert(w.clone(), k);
        }
    }
    // Mods also edit bins shared across WADs; unknown ones use everything learned.
    let mut all = FieldKinds::default();
    for k in kinds_by_wad.values() {
        all.merge(k);
    }
    let kinds_of = |wad: &str| kinds_by_wad.get(&base_wad_name(wad)).map(|k| k.as_ref()).unwrap_or(&all);

    let tmp = out_path.with_extension("fantome.part");
    let mut writer = zip::ZipWriter::new(File::create(&tmp).map_err(|e| e.to_string())?);
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut converted = 0;

    let result = (|| -> Result<(), String> {
        for i in 0..archive.len() {
            let (name, wad) = {
                let f = archive.by_index_raw(i).map_err(|e| e.to_string())?;
                (f.name().to_string(), if f.is_dir() { None } else { wad_of(f.name()) })
            };
            let replacement = match &wad {
                Some((wad_name, true)) => {
                    let mut bytes = Vec::new();
                    archive.by_index(i).map_err(|e| e.to_string())?.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
                    repair_wad(kinds_of(wad_name), bytes)?
                }
                Some((wad_name, false)) if name.to_lowercase().ends_with(".bin") => {
                    let mut bytes = Vec::new();
                    archive.by_index(i).map_err(|e| e.to_string())?.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
                    if is_bin(&bytes) { repair_bin(kinds_of(wad_name), &bytes) } else { None }
                }
                _ => None,
            };
            match replacement {
                Some((bytes, n)) => {
                    converted += n;
                    writer.start_file(name, stored).map_err(|e| e.to_string())?;
                    writer.write_all(&bytes).map_err(|e| e.to_string())?;
                }
                None => {
                    let f = archive.by_index_raw(i).map_err(|e| e.to_string())?;
                    writer.raw_copy_file(f).map_err(|e| e.to_string())?;
                }
            }
        }
        writer.finish().map_err(|e| e.to_string())?;
        Ok(())
    })();

    if let Err(e) = result {
        fs::remove_file(&tmp).ok();
        return Err(e);
    }

    let record = serde_json::to_vec(&CacheRecord { converted }).map_err(|e| e.to_string())?;
    if converted == 0 {
        fs::remove_file(&tmp).ok();
        fs::write(&record_path, record).ok();
        return Ok(Prepared { path: src.to_string(), converted: 0 });
    }
    fs::rename(&tmp, &out_path).map_err(|e| e.to_string())?;
    fs::write(&record_path, record).ok();
    Ok(Prepared { path: out_path.to_string_lossy().into_owned(), converted })
}
