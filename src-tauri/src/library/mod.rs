//! Downloadable SFZ libraries: catalog, install (download + extract), and
//! General MIDI program → instrument suggestions.

mod catalog;

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

pub use catalog::{Archive, CatalogInstrument, CatalogLibrary, CATALOG};

use crate::instruments::{InstrumentChoice, SfzPart};

const INSTALLED_MARKER: &str = ".midiss-installed";

/// Installs in progress (one at a time per library).
#[derive(Default)]
pub struct LibraryState {
    installing: Mutex<HashSet<String>>,
}

fn libraries_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("libraries"))
}

fn library_dir(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    Ok(libraries_dir(app)?.join(id))
}

pub fn is_installed(dir: &Path) -> bool {
    dir.join(INSTALLED_MARKER).exists()
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentEntry {
    pub id: String,
    pub name: String,
    pub library: String,
    pub library_name: String,
    /// Pass to `engine_set_track_instrument` as-is.
    pub choice: InstrumentChoice,
}

/// Locate an .sfz inside an installed library: at its catalog path, or —
/// if the archive's folder layout differs — by file name anywhere below.
fn find_sfz(dir: &Path, rel: &str) -> Option<PathBuf> {
    let direct = dir.join(rel);
    if direct.is_file() {
        return Some(direct);
    }
    let name = Path::new(rel).file_name()?;
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        for entry in std::fs::read_dir(&d).ok()?.flatten() {
            let p = entry.path();
            if p.is_dir() && depth < 4 {
                stack.push((p, depth + 1));
            } else if p.file_name() == Some(name) {
                return Some(p);
            }
        }
    }
    None
}

fn entry_for(lib: &CatalogLibrary, inst: &CatalogInstrument, dir: &Path) -> Option<InstrumentEntry> {
    let choice = if inst.split.is_empty() {
        InstrumentChoice::Sfz { path: find_sfz(dir, inst.sfz)?.to_string_lossy().into_owned(), gain_db: inst.gain_db }
    } else {
        let parts = inst
            .split
            .iter()
            .map(|p| Some(SfzPart { path: find_sfz(dir, p.sfz)?.to_string_lossy().into_owned(), lo: p.lo, hi: p.hi }))
            .collect::<Option<Vec<_>>>()?;
        InstrumentChoice::SfzSplit { parts, gain_db: inst.gain_db }
    };
    Some(InstrumentEntry {
        id: inst.id.to_string(),
        name: inst.name.to_string(),
        library: lib.id.to_string(),
        library_name: lib.name.to_string(),
        choice,
    })
}

/// The built-in synthesizer's presets, listed like an installed library's
/// instruments (always there: nothing to download).
pub fn builtin_instruments() -> Vec<InstrumentEntry> {
    midiss_synth::PRESETS.iter().map(synth_entry).collect()
}

fn synth_entry(p: &midiss_synth::Preset) -> InstrumentEntry {
    InstrumentEntry {
        id: format!("synth:{}", p.id),
        name: p.name.to_string(),
        library: "builtin-synth".into(),
        library_name: "内蔵シンセ".into(),
        choice: InstrumentChoice::Synth { preset: p.id.to_string() },
    }
}

#[tauri::command]
pub fn synth_presets() -> Vec<InstrumentEntry> {
    builtin_instruments()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStatus {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    license: &'static str,
    attribution: &'static str,
    homepage: &'static str,
    download_size: u64,
    installed: bool,
    installing: bool,
    instruments: Vec<InstrumentEntry>,
}

fn instruments_of(lib: &CatalogLibrary, dir: &Path) -> Vec<InstrumentEntry> {
    lib.instruments.iter().filter_map(|i| entry_for(lib, i, dir)).collect()
}

#[tauri::command]
pub fn library_list(app: AppHandle, state: State<'_, LibraryState>) -> Result<Vec<LibraryStatus>, String> {
    let installing = state.installing.lock().map_err(|_| "poisoned")?.clone();
    CATALOG
        .iter()
        .map(|lib| {
            let dir = library_dir(&app, lib.id)?;
            let installed = is_installed(&dir);
            Ok(LibraryStatus {
                id: lib.id,
                name: lib.name,
                description: lib.description,
                license: lib.license,
                attribution: lib.attribution,
                homepage: lib.homepage,
                download_size: lib.download_size,
                installed,
                installing: installing.contains(lib.id),
                instruments: if installed { instruments_of(lib, &dir) } else { Vec::new() },
            })
        })
        .collect()
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress<'a> {
    id: &'a str,
    /// "download" | "extract" | "done" | "error"
    phase: &'a str,
    received: u64,
    total: u64,
    message: Option<String>,
}

fn emit(app: &AppHandle, p: Progress<'_>) {
    let _ = app.emit("library-progress", p);
}

/// Download and extract a catalog library. Progress is reported through the
/// `library-progress` event; the command resolves when done.
#[tauri::command]
pub async fn library_install(app: AppHandle, state: State<'_, LibraryState>, id: String) -> Result<(), String> {
    let lib = CATALOG.iter().find(|l| l.id == id).ok_or("unknown library")?;
    {
        let mut busy = state.installing.lock().map_err(|_| "poisoned")?;
        if !busy.insert(id.clone()) {
            return Err("already installing".into());
        }
    }
    let result = install(&app, lib).await;
    state.installing.lock().map_err(|_| "poisoned")?.remove(&id);
    match &result {
        Ok(()) => emit(&app, Progress { id: lib.id, phase: "done", received: 0, total: 0, message: None }),
        Err(e) => {
            emit(&app, Progress { id: lib.id, phase: "error", received: 0, total: 0, message: Some(e.clone()) })
        }
    }
    result
}

async fn install(app: &AppHandle, lib: &'static CatalogLibrary) -> Result<(), String> {
    let base = libraries_dir(app)?;
    tokio::fs::create_dir_all(&base).await.map_err(|e| e.to_string())?;
    let archive_path = base.join(format!("{}.download", lib.id));
    download(app, lib, &archive_path).await?;

    emit(app, Progress { id: lib.id, phase: "extract", received: 0, total: 0, message: None });
    let staging = base.join(format!(".{}.staging", lib.id));
    let target = base.join(lib.id);
    let (a, s, t) = (archive_path.clone(), staging.clone(), target.clone());
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let _ = std::fs::remove_dir_all(&s);
        std::fs::create_dir_all(&s).map_err(|e| e.to_string())?;
        extract(lib.archive, &a, &s)?;
        let root = library_root(&s, lib.root)?;
        let _ = std::fs::remove_dir_all(&t);
        std::fs::rename(&root, &t).map_err(|e| e.to_string())?;
        std::fs::write(t.join(INSTALLED_MARKER), lib.url).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&s);
        let _ = std::fs::remove_file(&a);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn download(app: &AppHandle, lib: &CatalogLibrary, path: &Path) -> Result<(), String> {
    let response = reqwest::get(lib.url).await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("download failed: HTTP {}", response.status()));
    }
    let total = response.content_length().unwrap_or(lib.download_size);
    let mut file = File::create(path).map_err(|e| e.to_string())?;
    let mut received = 0u64;
    let mut last_report = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        received += chunk.len() as u64;
        if received - last_report >= 1 << 20 {
            last_report = received;
            emit(app, Progress { id: lib.id, phase: "download", received, total, message: None });
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    Ok(())
}

fn extract(kind: Archive, archive: &Path, into: &Path) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| e.to_string())?;
    match kind {
        Archive::Zip => {
            let mut zip = zip::ZipArchive::new(BufReader::new(file)).map_err(|e| e.to_string())?;
            // `extract` rejects entries escaping the target directory.
            zip.extract(into).map_err(|e| e.to_string())
        }
        Archive::TarGz => {
            let gz = flate2::read::GzDecoder::new(BufReader::new(file));
            // `unpack` refuses absolute paths and `..` components.
            tar::Archive::new(gz).unpack(into).map_err(|e| e.to_string())
        }
        Archive::TarXz => {
            let tar_path = archive.with_extension("tar");
            {
                let mut out = std::io::BufWriter::new(File::create(&tar_path).map_err(|e| e.to_string())?);
                lzma_rs::xz_decompress(&mut BufReader::new(file), &mut out).map_err(|e| format!("xz: {e:?}"))?;
                out.flush().map_err(|e| e.to_string())?;
            }
            let result = tar::Archive::new(BufReader::new(File::open(&tar_path).map_err(|e| e.to_string())?))
                .unpack(into)
                .map_err(|e| e.to_string());
            let _ = std::fs::remove_file(&tar_path);
            result
        }
    }
}

/// The directory to install: `root` inside the archive, or the archive's
/// single top-level folder when `root` is empty.
fn library_root(staging: &Path, root: &str) -> Result<PathBuf, String> {
    if !root.is_empty() {
        let p = staging.join(root);
        return if p.is_dir() { Ok(p) } else { Err(format!("archive has no folder {root}")) };
    }
    let entries: Vec<PathBuf> = std::fs::read_dir(staging)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .collect();
    match entries.as_slice() {
        [single] if single.is_dir() => Ok(single.clone()),
        _ => Ok(staging.to_path_buf()),
    }
}

#[tauri::command]
pub async fn library_remove(app: AppHandle, id: String) -> Result<(), String> {
    let lib = CATALOG.iter().find(|l| l.id == id).ok_or("unknown library")?;
    let dir = library_dir(&app, lib.id)?;
    tauri::async_runtime::spawn_blocking(move || match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(serde::Deserialize)]
pub struct TrackProgram {
    program: u8,
    percussion: bool,
}

/// Best installed SFZ instrument for each track's GM program, or `None` to
/// keep the SoundFont. Earlier catalog entries win ties.
#[tauri::command]
pub fn library_suggest(app: AppHandle, tracks: Vec<TrackProgram>) -> Result<Vec<Option<InstrumentEntry>>, String> {
    let mut installed = Vec::new();
    for lib in CATALOG {
        let dir = library_dir(&app, lib.id)?;
        if is_installed(&dir) {
            installed.push((lib, dir));
        }
    }
    Ok(tracks
        .iter()
        .map(|t| {
            installed.iter().find_map(|(lib, dir)| {
                let inst = lib
                    .instruments
                    .iter()
                    .find(|i| i.percussion == t.percussion && (t.percussion || i.programs.contains(&t.program)))?;
                entry_for(lib, inst, dir)
            })
            // GM's synth programs sound thin on a SoundFont; the built-in
            // synth plays them when no downloaded library does.
            .or_else(|| (!t.percussion).then(|| midiss_synth::for_program(t.program).map(synth_entry)).flatten())
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_tar_gz_and_picks_single_root() {
        let tmp = std::env::temp_dir().join(format!("midiss-lib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let archive = tmp.join("lib.tar.gz");
        {
            let gz = flate2::write::GzEncoder::new(File::create(&archive).unwrap(), flate2::Compression::fast());
            let mut builder = tar::Builder::new(gz);
            let data = b"<region> sample=a.wav\n";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, "My Library/Piano/piano.sfz", &data[..]).unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }
        let staging = tmp.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        extract(Archive::TarGz, &archive, &staging).unwrap();
        let root = library_root(&staging, "").unwrap();
        assert!(root.ends_with("My Library"));
        assert!(root.join("Piano/piano.sfz").exists());
        assert!(library_root(&staging, "nope").is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn catalog_is_consistent() {
        let mut ids = HashSet::new();
        for lib in CATALOG {
            assert!(ids.insert(lib.id), "duplicate library id {}", lib.id);
            assert!(lib.url.starts_with("https://"), "{}", lib.id);
            let mut inst_ids = HashSet::new();
            for i in lib.instruments {
                assert!(inst_ids.insert(i.id), "duplicate instrument {} in {}", i.id, lib.id);
                assert!(i.sfz.ends_with(".sfz") || !i.split.is_empty(), "{}", i.id);
                for p in i.split {
                    assert!(p.sfz.ends_with(".sfz") && p.lo <= p.hi, "{}", i.id);
                }
                assert!(i.percussion || !i.programs.is_empty(), "{} maps no program", i.id);
                assert!(i.programs.iter().all(|&p| p < 128));
            }
        }
    }
}

#[cfg(test)]
mod network_tests {
    //! Downloads real catalog entries (small ones only). Run explicitly:
    //! `cargo test --release -- --ignored catalog_downloads`
    //! Set `MIDISS_LIB_CACHE` to reuse downloaded archives between runs.

    use super::*;
    use crate::engine::player::Instrument;
    use crate::instruments::sfz::{SfzInstrument, SfzSplit};

    #[test]
    #[ignore]
    fn catalog_downloads_install_and_play() {
        let cache = std::env::var("MIDISS_LIB_CACHE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("midiss-lib-cache"));
        std::fs::create_dir_all(&cache).unwrap();
        let ids = [
            "freepats-gm-drums",
            "freepats-drawbar-organ",
            "freepats-church-organ",
            "freepats-steel-guitar",
            "freepats-tenor-sax",
            "karoryfer-growlybass",
        ];
        for id in ids {
            let lib = CATALOG.iter().find(|l| l.id == id).unwrap();
            let archive = cache.join(format!("{id}.download"));
            if !archive.exists() {
                let ok = std::process::Command::new("curl")
                    .args(["-sSLf", "-o"])
                    .arg(&archive)
                    .arg(lib.url)
                    .status()
                    .unwrap()
                    .success();
                assert!(ok, "download {id}");
            }
            let staging = cache.join(format!("{id}.staging"));
            let _ = std::fs::remove_dir_all(&staging);
            std::fs::create_dir_all(&staging).unwrap();
            extract(lib.archive, &archive, &staging).unwrap_or_else(|e| panic!("extract {id}: {e}"));
            let root = library_root(&staging, lib.root).unwrap();
            let entries = instruments_of(lib, &root);
            assert_eq!(entries.len(), lib.instruments.len(), "{id}: some .sfz not found under {}", root.display());
            for e in entries {
                let mut inst: Box<dyn Instrument> = match &e.choice {
                    InstrumentChoice::Sfz { path, .. } => Box::new(SfzInstrument::load(Path::new(path), 48_000).unwrap()),
                    InstrumentChoice::SfzSplit { parts, .. } => {
                        let parts: Vec<_> = parts.iter().map(|p| (p.lo, p.hi, PathBuf::from(&p.path))).collect();
                        Box::new(SfzSplit::load(&parts, 48_000).unwrap())
                    }
                    other => panic!("{other:?}"),
                };
                let is_drum = lib.instruments.iter().any(|i| i.id == e.id && i.percussion);
                let key = if is_drum { 38 } else { 60 };
                inst.note_on(0, key, 110);
                let (mut l, mut r) = (vec![0f32; 1024], vec![0f32; 1024]);
                let mut energy = 0.0;
                for _ in 0..20 {
                    inst.render(&mut l, &mut r);
                    energy += l.iter().chain(&r).map(|v| v * v).sum::<f32>();
                }
                eprintln!("{id}/{}: energy {energy:.3}", e.id);
                assert!(energy > 0.001, "{id}/{} silent", e.id);
            }
        }
    }
}

#[cfg(test)]
mod calibration {
    //! Prints `gain_db` values that match each installed catalog instrument
    //! to the SoundFont patch it replaces. Needs `MIDISS_SF2`.
    //! `cargo test --release -- --ignored calibrate_catalog --nocapture`

    use super::*;
    use crate::engine::player::Instrument;
    use crate::instruments::sfz::{SfzInstrument, SfzSplit};
    use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
    use std::sync::Arc;

    const SR: u32 = 48_000;
    const BLOCKS: usize = 40;

    fn test_keys(program: u8, percussion: bool) -> Vec<u8> {
        // Several keys across the typical range average out sample-to-sample
        // level differences.
        if percussion {
            return vec![36, 38, 42, 46];
        }
        match program {
            48..=51 | 44 | 45 => vec![48, 60, 67, 76],
            32..=39 | 43 | 47 | 58 | 70 => vec![36, 43, 50],
            9 | 12 | 13 | 72 | 73 | 79 => vec![72, 79, 84],
            _ => vec![48, 60, 67],
        }
    }

    fn sf2_energy(sf: &Arc<SoundFont>, program: u8, percussion: bool, keys: &[u8]) -> f32 {
        let mut s = Synthesizer::new(sf, &SynthesizerSettings::new(SR as i32)).unwrap();
        let ch = if percussion { 9 } else { 0 };
        s.process_midi_message(ch, 0xC0, i32::from(program), 0);
        for &k in keys {
            s.note_on(ch, i32::from(k), 100);
        }
        let (mut l, mut r) = (vec![0f32; 1024], vec![0f32; 1024]);
        (0..BLOCKS)
            .map(|_| {
                s.render(&mut l, &mut r);
                l.iter().chain(&r).map(|v| v * v).sum::<f32>()
            })
            .sum()
    }

    fn sfz_energy(choice: &InstrumentChoice, keys: &[u8]) -> f32 {
        let mut inst: Box<dyn Instrument> = match choice {
            InstrumentChoice::Sfz { path, .. } => Box::new(SfzInstrument::load(Path::new(path), SR).unwrap()),
            InstrumentChoice::SfzSplit { parts, .. } => {
                let parts: Vec<_> = parts.iter().map(|p| (p.lo, p.hi, PathBuf::from(&p.path))).collect();
                Box::new(SfzSplit::load(&parts, SR).unwrap())
            }
            _ => unreachable!(),
        };
        for &k in keys {
            inst.note_on(0, k, 100);
        }
        let (mut l, mut r) = (vec![0f32; 1024], vec![0f32; 1024]);
        (0..BLOCKS)
            .map(|_| {
                inst.render(&mut l, &mut r);
                l.iter().chain(&r).map(|v| v * v).sum::<f32>()
            })
            .sum()
    }

    #[test]
    #[ignore]
    fn calibrate_catalog() {
        let Ok(sf2) = std::env::var("MIDISS_SF2") else { return };
        let sf = Arc::new(SoundFont::new(&mut BufReader::new(File::open(sf2).unwrap())).unwrap());
        let base = PathBuf::from(std::env::var("HOME").unwrap())
            .join("Library/Application Support/com.midiss.app/libraries");
        for lib in CATALOG {
            let dir = base.join(lib.id);
            if !is_installed(&dir) {
                eprintln!("{}: not installed", lib.id);
                continue;
            }
            for inst in lib.instruments {
                let Some(entry) = entry_for(lib, inst, &dir) else {
                    eprintln!("{}/{}: MISSING", lib.id, inst.id);
                    continue;
                };
                let program = inst.programs.first().copied().unwrap_or(0);
                let keys = test_keys(program, inst.percussion);
                let reference = sf2_energy(&sf, program, inst.percussion, &keys);
                let measured = sfz_energy(&entry.choice, &keys);
                let gain = 10.0 * (reference / measured.max(1e-9)).log10();
                eprintln!(
                    "CAL {}/{} program {program}: sf2 {reference:.1} sfz {measured:.1} -> gain {gain:+.1} dB (now {:+.1})",
                    lib.id, inst.id, inst.gain_db
                );
            }
        }
    }
}

/* ---------- for the MCP control socket ---------- */

/// Everything this machine can play a part through: the SFZ instruments of
/// installed libraries, and the audio plugins found on the system.
///
/// Listed here rather than in the UI because what is installed is Rust's to
/// know, and the socket has to answer straight away.
pub fn installed(app: &AppHandle) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for lib in CATALOG.iter() {
        let Ok(dir) = library_dir(app, lib.id) else { continue };
        if !is_installed(&dir) {
            continue;
        }
        for entry in instruments_of(lib, &dir) {
            out.push(serde_json::json!({
                "id": entry.id,
                "kind": "sfz",
                "name": format!("{} — {}", entry.library_name, entry.name),
            }));
        }
    }
    for p in midiss_synth::PRESETS {
        out.push(serde_json::json!({
            "id": format!("synth:{}", p.id),
            "kind": "synth",
            "name": format!("内蔵シンセ — {}", p.name),
            "description": p.description,
        }));
    }
    #[cfg(target_os = "macos")]
    for plugin in crate::instruments::audio_unit::list_instruments() {
        if let Ok(value) = serde_json::to_value(&plugin) {
            let id = value.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_owned();
            let name = value.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_owned();
            out.push(serde_json::json!({ "id": format!("au:{id}"), "kind": "plugin", "name": name }));
        }
    }
    out
}

/// Play `track` of the song on screen through the instrument called `id`.
///
/// This is a playback setting, not part of the song: it changes what the
/// engine sounds, and nothing is written to the file.
pub fn use_instrument(app: &AppHandle, track: usize, id: &str) -> Result<String, String> {
    let (choice, label) = match id.strip_prefix("au:") {
        Some(plugin) => (InstrumentChoice::AudioUnit { id: plugin.to_owned() }, id.to_owned()),
        None if id == "sf2" || id.eq_ignore_ascii_case("default") => {
            (InstrumentChoice::Sf2, "内蔵の SoundFont".to_owned())
        }
        None => {
            let entry = find_instrument(app, id)
                .ok_or_else(|| format!("{id}: そういう音源は入っていません。list_playback_instruments で確認してください。"))?;
            let label = format!("{} — {}", entry.library_name, entry.name);
            (entry.choice, label)
        }
    };
    let handle = app.state::<crate::engine::EngineHandle>();
    crate::engine::assign_instrument(&handle, track, choice)?;
    Ok(format!("トラック{track} を「{label}」で鳴らします。"))
}

fn find_instrument(app: &AppHandle, id: &str) -> Option<InstrumentEntry> {
    let preset = id.strip_prefix("synth:").unwrap_or(id);
    if let Some(p) = midiss_synth::PRESETS.iter().find(|p| p.id == preset) {
        return Some(synth_entry(p));
    }
    for lib in CATALOG.iter() {
        let dir = library_dir(app, lib.id).ok()?;
        if !is_installed(&dir) {
            continue;
        }
        if let Some(found) = instruments_of(lib, &dir).into_iter().find(|e| e.id == id) {
            return Some(found);
        }
    }
    None
}
