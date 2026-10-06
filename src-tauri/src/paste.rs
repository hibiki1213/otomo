//! Paste: a range copied from the piano roll of one song, set into another
//! (or the same) at a bar and beat.
//!
//! A person's paste is saved straight away, keeping the version it replaced
//! so "Undo Edit" takes it back — unless Claude has edits waiting on that
//! song, in which case it joins them, so there are never two versions in
//! play at once. The notes themselves move as `otomo_score::Clip`s, the
//! same as Claude's copy between songs.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::i18n::tr;
use crate::songs::{SongLibrary, SongRecord};
use otomo_score::{Score, Voice};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PastePart {
    /// Track in the song copied from.
    from: usize,
    /// Track in the song pasted into; none makes a new one.
    to: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteRequest {
    from_id: i64,
    /// Copied while Claude's staged version was playing.
    #[serde(default)]
    from_staged: bool,
    /// The copied range, in quarter notes from the start of its song.
    start_beat: f64,
    end_beat: f64,
    /// Lowest and highest key of the range, both included.
    lo: u8,
    hi: u8,
    parts: Vec<PastePart>,
    to_id: i64,
    /// Where it goes, in quarter notes from the start — worked out by the UI
    /// from the bar and beat it shows, so it lands where the person sees it.
    at_beat: f64,
    /// That bar's number, for the note on a staged edit.
    at_bar: u32,
    #[serde(default)]
    transpose: i32,
    /// Layer onto what is there instead of replacing it.
    #[serde(default)]
    merge: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteResult {
    record: SongRecord,
    /// It joined Claude's staged edit rather than being saved.
    staged: bool,
    /// Where the paste starts, for playing from there.
    at_seconds: f64,
    notes: usize,
}

#[tauri::command]
pub async fn songs_paste(library: State<'_, SongLibrary>, request: PasteRequest) -> Result<PasteResult, String> {
    let (_, from_bytes) = library.source(request.from_id, request.from_staged)?;
    let staged = library.has_pending(request.to_id);
    let (_, to_bytes) = library.source(request.to_id, staged)?;
    let source = Score::parse(&from_bytes)?;
    let mut score = Score::parse(&to_bytes)?;

    let tick = |beats: f64, ppq: u32| (beats.max(0.0) * f64::from(ppq)).round() as u32;
    let (start, end) = (tick(request.start_beat, source.ppq), tick(request.end_beat, source.ppq));
    let at = tick(request.at_beat, score.ppq);

    let mut notes = 0;
    for part in &request.parts {
        let clip = source.clip(part.from, start, end, (request.lo, request.hi), Voice::All)?;
        if clip.note_count() == 0 {
            continue;
        }
        notes += clip.note_count();
        score.paste(&clip, part.to, at, request.transpose, request.merge)?;
    }
    if notes == 0 {
        return Err(tr!("選んだ範囲に音がありません", "There are no notes in the copied range"));
    }

    let data = score.to_smf()?;
    let record = if staged {
        let change = format!("Pasted {notes} notes at bar {} (by hand)", request.at_bar);
        library.stage(request.to_id, &data, &change)?;
        library.record(request.to_id)?
    } else {
        library.commit(request.to_id, &data, "paste", notes)?
    };
    Ok(PasteResult { record, staged, at_seconds: score.tick_seconds(at), notes })
}
