//! Sheet music out of the library: a song, or some of its tracks, as
//! MusicXML. The notation itself is `otomo_score::to_musicxml`; the export
//! dialog engraves the same text into PDF pages (see `pdf.rs`).

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::i18n::tr;
use crate::songs::SongLibrary;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreRequest {
    id: i64,
    /// The version Claude staged rather than the saved one.
    #[serde(default)]
    staged: bool,
    /// Track numbers as the player lists them, in the order to print.
    tracks: Vec<usize>,
    /// Transposing instruments at the pitch their players read.
    transposed: bool,
    /// Runs of empty bars as one rest, for a player's part.
    multi_rests: bool,
    /// Only the first bars, for a quick preview.
    #[serde(default)]
    max_bars: Option<u32>,
}

/// A track as the export dialog lists it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreTrack {
    index: usize,
    name: String,
    /// The General MIDI instrument, or "Drums".
    instrument: String,
    percussion: bool,
    notes: usize,
}

#[tauri::command]
pub async fn score_tracks(
    library: State<'_, SongLibrary>,
    id: i64,
    staged: Option<bool>,
) -> Result<Vec<ScoreTrack>, String> {
    let (_, bytes) = library.source(id, staged.unwrap_or(false))?;
    let score = otomo_score::Score::parse(&bytes)?;
    Ok(score
        .tracks
        .iter()
        .enumerate()
        .map(|(index, t)| ScoreTrack {
            index,
            name: t.display_name(),
            instrument: if t.is_percussion() { "Drums".into() } else { otomo_score::program_name(t.program).into() },
            percussion: t.is_percussion(),
            notes: t.notes.len(),
        })
        .collect())
}

#[tauri::command]
pub async fn score_musicxml(library: State<'_, SongLibrary>, request: ScoreRequest) -> Result<String, String> {
    let (title, bytes) = library.source(request.id, request.staged)?;
    let fail = |e: String| tr!("楽譜を書き出せませんでした: {e}", "Couldn't write the score: {e}");
    let score = otomo_score::Score::parse(&bytes).map_err(fail)?;
    let options = otomo_score::ExportOptions {
        title,
        tracks: request.tracks,
        transposed: request.transposed,
        multi_rests: request.multi_rests,
        max_bars: request.max_bars,
    };
    otomo_score::to_musicxml(&score, &options).map_err(fail)
}

/// Save text the export dialog made (a MusicXML file) where the person chose.
#[tauri::command]
pub async fn export_write_text(path: String, contents: String) -> Result<(), String> {
    tokio::fs::write(&path, contents)
        .await
        .map_err(|e| tr!("{path} に保存できませんでした: {e}", "Couldn't save {path}: {e}"))
}
