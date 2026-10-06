//! Sheet music out of the library: a song, or some of its tracks, as
//! MusicXML. The notation itself is `otomo_score::to_musicxml`; the export
//! dialog engraves the same text into PDF pages (see `pdf.rs`). Also the
//! chord symbols the piano roll's chord lane shows when a file has none.

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

/// A chord symbol on the piano roll's timeline.
#[derive(Serialize)]
pub struct ChordAt {
    /// Seconds from the start, where the harmony begins.
    time: f64,
    /// "F", "Bbmaj7", "G7/B"; "—" for silence, "?" for nothing nameable.
    name: String,
}

/// The harmony of the whole song beat by beat, read the way Claude's
/// `harmony` tool reads it.
#[tauri::command]
pub async fn score_chords(
    library: State<'_, SongLibrary>,
    id: i64,
    staged: Option<bool>,
) -> Result<Vec<ChordAt>, String> {
    let (_, bytes) = library.source(id, staged.unwrap_or(false))?;
    // Every beat weighs every note: a long orchestral score takes a moment.
    tauri::async_runtime::spawn_blocking(move || {
        let score = otomo_score::Score::parse(&bytes)?;
        let bars = score.bars();
        let tracks: Vec<usize> = (0..score.tracks.len()).collect();
        Ok(otomo_score::analyse_harmony(&score, &bars, &tracks, 1, bars.count)
            .into_iter()
            .map(|chord| {
                // The beat as `analyse` counts it: the bar split by its numerator.
                let beats = u32::from(bars.time_signature(chord.bar).0.max(1));
                let beat_ticks = (bars.bar_ticks(chord.bar) / beats).max(1);
                let tick = bars.bar_start(chord.bar) + ((chord.beat - 1.0) * f64::from(beat_ticks)).round() as u32;
                ChordAt { time: score.tick_seconds(tick), name: chord.name }
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Save text the export dialog made (a MusicXML file) where the person chose.
#[tauri::command]
pub async fn export_write_text(path: String, contents: String) -> Result<(), String> {
    tokio::fs::write(&path, contents)
        .await
        .map_err(|e| tr!("{path} に保存できませんでした: {e}", "Couldn't save {path}: {e}"))
}
