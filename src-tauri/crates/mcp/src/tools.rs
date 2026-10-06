//! The tools themselves.
//!
//! Everything answers in plain text rather than JSON: it is what the model
//! reads best, and it keeps long passages affordable. Positions are always
//! bars and beats — never ticks, never seconds — so that what the model says
//! and what a musician says are the same thing.

use otomo_score::{analyse_harmony, program_name, read_musicxml, render_bars, suggest_grid, Grid, Score};
use serde_json::{json, Value};

use crate::control;
use crate::edit_tools;
use crate::fetch::{self, Fetched};
use crate::library::{data_dir, Library, Song};
use crate::pending;
use crate::picture;
use crate::rpc::{need_i64, need_str, opt_i64, opt_str, Handler, Reply, Tool};

/// Reading a whole symphony in one call would drown the conversation; this is
/// roughly a page of music.
const MAX_BARS: u32 = 64;
const MAX_SONGS: i64 = 50;
const MAX_TRACKS: usize = 40;
/// Downloaded text comes back a page at a time: a MusicXML score runs to
/// megabytes, an ABC tune to a few hundred characters.
const PAGE_CHARS: usize = 30_000;
/// Pages of a PDF score shown at once: each is a full-page picture.
const MAX_PAGES: usize = 3;

pub struct Server;

impl Handler for Server {
    fn server_name(&self) -> &'static str {
        "otomo"
    }

    fn server_version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn tools(&self) -> Vec<Tool> {
        vec![
            Tool {
                name: "list_songs",
                description: "List the songs in the Otomo library. Use song_id with the other tools.",
                schema: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Filter by part of the title" }
                    }
                }),
            },
            Tool {
                name: "download",
                description: "Fetch a file from a URL. For music that already exists (national anthems, classical pieces, \
                              folk songs, standards, etc.), do not write it from memory: search the web for sheet music or MIDI \
                              and import it here.\n\
                              - MusicXML (.musicxml / .xml / .mxl) → imports the notes of the score as they are, as a song. The most accurate option.\n\
                              - MIDI (.mid) → imports it into the library as a new song.\n\
                              Both return the same overview as get_song; check the notes with read_bars before reworking the \
                              instrumentation or arrangement.\n\
                              - PDF or image (PNG / JPEG) sheet music → returns the pages as images. Read them and transcribe \
                              them with create_song and write_score. Use this when no MusicXML or MIDI can be found. \
                              Choose the pages to read with pages (up to 3 at a time).\n\
                              - Text (ABC notation, lyrics, etc.) → returns the contents unprocessed. Read the rest of a long text with offset.\n\
                              Read HTML pages with WebFetch. For SVG sheet music, use the URL of a version converted to PNG.",
                schema: json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "URL of the file itself (http / https)" },
                        "title": { "type": "string", "description": "Title for the imported song (defaults to the score's title or the file name)" },
                        "pages": { "type": "string", "description": "Pages of a PDF to read: \"1-2\" (default), \"3\", etc." },
                        "offset": { "type": "integer", "description": "Character position to start from when reading the rest of a text" }
                    },
                    "required": ["url"]
                }),
            },
            Tool {
                name: "get_song",
                description: "Overview of a song: tracks (instrument, range, note count), key, time signature, tempo, \
                              and number of bars. Look at this first, before reading the notes with read_bars.",
                schema: json!({
                    "type": "object",
                    "properties": { "song_id": { "type": "integer" } },
                    "required": ["song_id"]
                }),
            },
            Tool {
                name: "read_bars",
                description: format!(
                    "Read the notes in a range of bars. One bar per line, each note written as `pitch@beat:length`: \
                     the beat counts from 1 within the bar, and the length is in beats. Notes that start together \
                     and have the same length are grouped as `[C4 E4 G4]`. MIDI recorded from a human performance \
                     is snapped to a grid that suits the passage (change it with quantize). \
                     Up to {MAX_BARS} bars per call."
                )
                .leak(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "song_id": { "type": "integer" },
                        "from_bar": { "type": "integer", "description": "Counts from 1" },
                        "to_bar": { "type": "integer" },
                        "tracks": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "description": "Track numbers from get_song. Omit for all tracks"
                        },
                        "velocity": { "type": "boolean", "description": "Also show velocities (default: false)" },
                        "quantize": {
                            "type": "string",
                            "description": "Grid to snap positions and lengths to. Omit to pick one that suits the passage automatically. \"1/16\", \"1/12\" (triplets), or \"off\" for no snapping"
                        }
                    },
                    "required": ["song_id", "from_bar", "to_bar"]
                }),
            },
            Tool {
                name: "harmony",
                description: "Read the harmony of a range of bars as chord symbols. \
                              In a piece with many parts, this is an order of magnitude lighter than read_bars \
                              for grasping what a passage is doing. Chords are identified beat by beat, and a chord \
                              that carries on is merged into one.",
                schema: json!({
                    "type": "object",
                    "properties": {
                        "song_id": { "type": "integer" },
                        "from_bar": { "type": "integer" },
                        "to_bar": { "type": "integer" },
                        "tracks": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "description": "Omit for all tracks (percussion is excluded)"
                        }
                    },
                    "required": ["song_id", "from_bar", "to_bar"]
                }),
            },
            Tool {
                name: "play",
                description: "Play this song in the running Otomo app, so the user can hear it. \
                              to_bar ends the range and loop repeats it (for fixing and listening again). \
                              With tracks, only those parts play (solo).",
                schema: json!({
                    "type": "object",
                    "properties": {
                        "song_id": { "type": "integer" },
                        "from_bar": { "type": "integer" },
                        "to_bar": { "type": "integer", "description": "Play up to this bar, then stop" },
                        "loop": { "type": "boolean", "description": "Repeat the range" },
                        "tracks": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "description": "Play only these parts. Omit for all"
                        }
                    },
                    "required": ["song_id"]
                }),
            },
            Tool {
                name: "list_playback_instruments",
                description: "List the sounds available in Otomo: the built-in synth, SFZ libraries, SFZ instruments \
                              the user added themselves (\"My Sounds\"), and AudioUnit plugins. The built-in synth has EDM sounds \
                              (supersaw, pluck, 808, wobble bass, EDM drums, etc.). Use this when a part should sound \
                              better than the General MIDI sounds. Requires Otomo to be running.",
                schema: json!({ "type": "object", "properties": {} }),
            },
            Tool {
                name: "set_playback_instrument",
                description: "Play a track of the song currently open through the given sound. \
                              This is a playback setting; the song data (MIDI) does not change. \
                              The setting is remembered per song, so the next time this song is opened it plays \
                              with the same sound.",
                schema: json!({
                    "type": "object",
                    "properties": {
                        "track": { "type": "integer" },
                        "instrument": { "type": "string", "description": "An id from list_playback_instruments" }
                    },
                    "required": ["track", "instrument"]
                }),
            },
            Tool {
                name: "stop",
                description: "Stop playback.",
                schema: json!({ "type": "object", "properties": {} }),
            },
        ]
        .into_iter()
        .chain(edit_tools::tools())
        .collect()
    }

    fn call(&mut self, name: &str, args: &Value) -> Result<Reply, String> {
        match name {
            "download" => download(&data_dir()?, args),
            _ => answer(name, args).map(Reply::from),
        }
    }
}

/// Every tool that answers in words alone.
fn answer(name: &str, args: &Value) -> Result<String, String> {
    let dir = data_dir()?;
    match name {
        "list_songs" => {
            let library = Library::open(&dir)?;
            Ok(list_songs(&library, opt_str(args, "query").as_deref())?)
        }
        "get_song" => {
            let library = Library::open(&dir)?;
            let (song, score) = load(&library, &dir, need_i64(args, "song_id")?)?;
            Ok(describe(&song, &score) + &edit_tools::describe_staged(&dir, &song).unwrap_or_default())
        }
        "read_bars" => {
            let library = Library::open(&dir)?;
            let (song, score) = load(&library, &dir, need_i64(args, "song_id")?)?;
            read(&song, &score, args)
        }
        "harmony" => {
            let library = Library::open(&dir)?;
            let (song, score) = load(&library, &dir, need_i64(args, "song_id")?)?;
            harmony(&song, &score, args)
        }
        "play" => {
            let library = Library::open(&dir)?;
            let (song, score) = load(&library, &dir, need_i64(args, "song_id")?)?;
            let bars = score.bars();
            let bar = opt_i64(args, "from_bar").unwrap_or(1).max(1) as u32;
            let seconds = score.tick_seconds(bars.bar_start(bar));
            let until = opt_i64(args, "to_bar").map(|to| {
                let to = (to.max(bar as i64) as u32).min(bars.count);
                score.tick_seconds(bars.bar_start(to) + bars.bar_ticks(to))
            });
            let looping = args.get("loop").and_then(Value::as_bool).unwrap_or(false);
            let solo = match args.get("tracks") {
                Some(_) => Some(wanted_tracks(&score, args)?),
                None => None,
            };
            let staged = pending::staged(&dir, song.id).is_some();
            control::play(&dir, song.id, seconds, until, looping, solo.clone(), staged)?;
            Ok(format!(
                "Playing \"{}\"{} from bar {bar} ({}){}{}.",
                song.title,
                if staged { " (the version with the pending edits)" } else { "" },
                clock(seconds),
                match (opt_i64(args, "to_bar"), looping) {
                    (Some(to), true) => format!(" to bar {to}, on a loop"),
                    (Some(to), false) => format!(" to bar {to}"),
                    (None, _) => String::new(),
                },
                match &solo {
                    Some(list) if !list.is_empty() => format!(
                        " (only {})",
                        list.iter()
                            .filter_map(|i| score.tracks.get(*i))
                            .map(track_label)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    _ => String::new(),
                }
            ))
        }
        "list_playback_instruments" => {
            let list = control::instruments(&dir)?;
            Ok(render_instruments(&list))
        }
        "set_playback_instrument" => {
            let reply = control::set_playback(
                &dir,
                usize::try_from(need_i64(args, "track")?).map_err(|_| "track must be 0 or more")?,
                need_str(args, "instrument")?,
            )?;
            Ok(reply
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Switched the sound.")
                .to_string())
        }
        "stop" => {
            control::stop(&dir)?;
            Ok("Stopped playback.".into())
        }
        other if edit_tools::is_edit(other) => {
            let library = Library::open(&dir)?;
            edit_tools::call(other, args, &library, &dir)
        }
        other => Err(format!("Unknown tool: {other}")),
    }
}

/// The song and the version to look at — the staged one when an edit is
/// waiting, so that reading back shows what was just written.
fn load(library: &Library, dir: &std::path::Path, id: i64) -> Result<(Song, Score), String> {
    let song = library.get(id)?;
    let score = pending::current(library, dir, &song)?;
    Ok((song, score))
}

/// A score file becomes a song; a PDF or a scan comes back as pictures to
/// read; a text file is read back as it is.
fn download(dir: &std::path::Path, args: &Value) -> Result<Reply, String> {
    let url = need_str(args, "url")?;
    let named = opt_str(args, "title").filter(|t| !t.trim().is_empty());
    match fetch::get(url)? {
        Fetched::Midi(bytes) => {
            // Checked before it is stored, so a broken file never reaches the library.
            let score = Score::parse(&bytes).map_err(|e| format!("Could not read the file as MIDI: {e}"))?;
            let title = named.or_else(|| fetch::title_from(url)).unwrap_or_else(|| "Downloaded song".into());
            import(dir, &title, &bytes, &score, url, &[])
        }
        Fetched::MusicXml(bytes) => {
            let read = read_musicxml(&bytes)?;
            let smf = read.score.to_smf()?;
            // Read back from the MIDI, so what is described is what was stored.
            let score = Score::parse(&smf)?;
            let title = named
                .or(read.title)
                .or_else(|| fetch::title_from(url))
                .unwrap_or_else(|| "Downloaded song".into());
            import(dir, &title, &smf, &score, url, &read.remarks)
        }
        Fetched::Pdf(bytes) => {
            let (first, last) = page_range(args)?;
            let pages = picture::pdf_pages(&bytes, first, last)?;
            let shown = pages.images.iter().map(|(n, _)| *n);
            let (from, to) = (shown.clone().min().unwrap_or(first), shown.max().unwrap_or(first));
            let mut text = format!(
                "{url} — PDF sheet music, pages {from}–{to} of {}.\n\n{READ_BY_EYE}",
                pages.count
            );
            if to < pages.count {
                let next = (to + 1, (to + MAX_PAGES).min(pages.count));
                text.push_str(&format!("\nRead on with pages: \"{}-{}\".", next.0, next.1));
            }
            Ok(Reply { text, images: pages.images.into_iter().map(|(_, png)| ("image/png", png)).collect() })
        }
        Fetched::Scan(bytes) => Ok(Reply {
            text: format!("{url} — an image of sheet music.\n\n{READ_BY_EYE}"),
            images: vec![("image/png", picture::scan(&bytes)?)],
        }),
        Fetched::Text(text) => Ok(text_page(url, &text, opt_i64(args, "offset").unwrap_or(0)).into()),
        Fetched::Html => Err("This is an HTML page. Read it with WebFetch, find a link to the score file itself \
                              (.musicxml / .mxl / .mid / .pdf, etc.), and pass that to download."
            .into()),
        Fetched::Svg => Err("SVG sheet music can't be read. Use the URL of a version converted to PNG \
                             (on Wikimedia Commons: upload.wikimedia.org/…/thumb/…/1600px-….svg.png)."
            .into()),
        Fetched::Other(kind) => Err(format!(
            "Can't read this file ({kind}). Give sheet music as MusicXML, MIDI, PDF or PNG / JPEG, or text such as ABC notation."
        )),
    }
}

/// What to do with a score that arrives as pictures.
const READ_BY_EYE: &str = "Read it and transcribe it.\n\
    - First establish the key signature, time signature, tempo, parts, and whether it starts with a pickup.\n\
    - Write it a few bars at a time with create_song → add_tracks → write_score. Write transposing instruments at concert pitch.\n\
    - Do not fill in anything you can't read or aren't sure of by guessing; say so in your reply.";

fn import(
    dir: &std::path::Path,
    title: &str,
    smf: &[u8],
    score: &Score,
    url: &str,
    remarks: &[String],
) -> Result<Reply, String> {
    let song = Library::open(dir)?.create(title, smf)?;
    let mut text = format!("Imported as \"{}\" (song_id {}, source {url}).\n", song.title, song.id);
    for remark in remarks {
        text.push_str(&format!("Note: {remark}\n"));
    }
    text.push('\n');
    text.push_str(&describe(&song, score));
    Ok(text.into())
}

/// "3" or "3-4"; the first two pages when not given, and never more than
/// MAX_PAGES at once.
fn page_range(args: &Value) -> Result<(usize, usize), String> {
    let text = opt_str(args, "pages").unwrap_or_else(|| "1-2".into());
    let (a, b) = text.split_once(['-', '–']).unwrap_or((&text, &text));
    let bad = || format!("{text}: write pages like \"1-2\" or \"3\"");
    let first: usize = a.trim().parse().map_err(|_| bad())?;
    let last: usize = b.trim().parse().map_err(|_| bad())?;
    if first == 0 || last < first {
        return Err(bad());
    }
    Ok((first, last.min(first + MAX_PAGES - 1)))
}

fn text_page(url: &str, text: &str, offset: i64) -> String {
    let total = text.chars().count();
    let from = (offset.max(0) as usize).min(total);
    let body: String = text.chars().skip(from).take(PAGE_CHARS).collect();
    let to = from + body.chars().count();
    let mut out = if from == 0 && to == total {
        format!("{url} ({total} characters)\n\n")
    } else {
        format!("{url} (characters {from}–{to} of {total})\n\n")
    };
    out.push_str(&body);
    if to < total {
        out.push_str(&format!("\n\n…There is more. Read on with offset: {to}."));
    }
    out
}

fn clock(seconds: f64) -> String {
    format!("{}:{:02}", seconds as u64 / 60, seconds as u64 % 60)
}

fn list_songs(library: &Library, query: Option<&str>) -> Result<String, String> {
    let songs = library.list(query, MAX_SONGS)?;
    if songs.is_empty() {
        return Ok(match query {
            Some(q) => format!("No songs match \"{q}\"."),
            None => "The library is empty. Songs are added when a MIDI file is opened in Otomo.".into(),
        });
    }
    let mut out = String::from("song_id  length tracks    title\n");
    for s in &songs {
        out.push_str(&format!(
            "{:<8} {:<6} {:<9} {}{}\n",
            s.id,
            if s.duration > 0.0 { clock(s.duration) } else { "-".into() },
            if s.track_count > 0 { s.track_count.to_string() } else { "-".into() },
            if s.favorite { "★ " } else { "" },
            s.title,
        ));
    }
    Ok(out)
}

fn describe(song: &Song, score: &Score) -> String {
    let bars = score.bars();
    let (lo, hi) = score.bpm_range();
    let mut out = format!("{} ({})\n", song.title, song.file_name);
    out.push_str(&format!(
        "Length {} / {} bars / tempo {} / {}\n",
        clock(score.duration()),
        bars.count,
        if lo == hi { format!("{lo} BPM") } else { format!("{lo}–{hi} BPM") },
        signature_summary(score, &bars),
    ));
    if let Some(k) = score.key_signatures.first() {
        out.push_str(&format!("Key {}\n", k.name()));
    }
    if let Some(beats) = score.looks_like_a_pickup() {
        out.push_str(&format!(
            "Note: the first note starts partway through bar 1 ({beats} {} before the end of the bar). \
             If it is a pickup, use set_pickup({beats}).\n",
            if beats == 1.0 { "beat" } else { "beats" }
        ));
    }
    for line in score.text.iter().take(3) {
        let short: String = line.chars().take(160).collect();
        let ellipsis = if line.chars().count() > 160 { "…" } else { "" };
        out.push_str(&format!("Text: {short}{ellipsis}\n"));
    }
    // A file that names its own sections is the cheapest map of itself.
    if !score.markers.is_empty() {
        out.push_str("\nSections: ");
        let marks: Vec<String> = score
            .markers
            .iter()
            .take(24)
            .map(|(tick, name)| format!("b{} {name}", bars.position(*tick).0))
            .collect();
        out.push_str(&marks.join(" / "));
        if score.markers.len() > 24 {
            out.push_str(&format!(" …and {} more", score.markers.len() - 24));
        }
        out.push('\n');
    }

    out.push_str("\n#    instrument                 notes  range     bars     name\n");
    for (i, track) in score.tracks.iter().enumerate().take(MAX_TRACKS) {
        let lo_key = track.notes.iter().map(|n| n.key).min().unwrap_or(0);
        let hi_key = track.notes.iter().map(|n| n.key).max().unwrap_or(0);
        let first = bars.position(track.notes.first().map_or(0, |n| n.start)).0;
        let last = bars.position(track.notes.iter().map(|n| n.start).max().unwrap_or(0)).0;
        out.push_str(&format!(
            "{:<4} {:<26} {:<6} {:<9} {:<8} {}\n",
            i,
            if track.is_percussion() {
                "Percussion".to_string()
            } else {
                program_name(track.program).to_string()
            },
            track.notes.len(),
            format!("{}–{}", otomo_score::note_name(lo_key), otomo_score::note_name(hi_key)),
            format!("{first}–{last}"),
            track.name,
        ));
    }
    if score.tracks.len() > MAX_TRACKS {
        out.push_str(&format!("…and {} more tracks\n", score.tracks.len() - MAX_TRACKS));
    }
    out
}

/// The harmony of a passage, as chord symbols.
fn harmony(song: &Song, score: &Score, args: &Value) -> Result<String, String> {
    let bars = score.bars();
    let from = need_i64(args, "from_bar")?.max(1) as u32;
    let to = need_i64(args, "to_bar")?.max(from as i64) as u32;
    if from > bars.count {
        return Err(format!("This song ends at bar {}.", bars.count));
    }
    // Chords are compact, so a wider window than read_bars is affordable.
    if to - from + 1 > MAX_BARS * 4 {
        return Err(format!("At most {} bars can be read at a time.", MAX_BARS * 4));
    }
    let tracks = wanted_tracks(score, args)?;

    let mut out = format!("{} — harmony, bars {from}–{}\n", song.title, to.min(bars.count));
    let mut line = String::new();
    let mut at_bar = 0u32;
    for chord in analyse_harmony(score, &bars, &tracks, from, to) {
        if chord.bar != at_bar {
            if !line.is_empty() {
                out.push_str(&line);
                out.push('\n');
            }
            line = format!("b{:<4}", chord.bar);
            at_bar = chord.bar;
        }
        if chord.beat > 1.0 {
            line.push_str(&format!(" {}:{}", chord.name, chord.beat));
        } else {
            line.push_str(&format!(" {}", chord.name));
        }
    }
    if !line.is_empty() {
        out.push_str(&line);
        out.push('\n');
    }
    Ok(out)
}

/// The app answers with a list of what it can play a part through.
fn render_instruments(reply: &Value) -> String {
    let items = reply.get("instruments").and_then(Value::as_array);
    let Some(items) = items.filter(|i| !i.is_empty()) else {
        return "Otomo has no additional sounds installed (only the built-in General MIDI sounds).".into();
    };
    let mut out = format!("{:<31} {:<11} {}\n", "id", "kind", "name");
    for item in items {
        out.push_str(&format!(
            "{:<31} {:<11} {}\n",
            item.get("id").and_then(Value::as_str).unwrap_or("-"),
            item.get("kind").and_then(Value::as_str).unwrap_or("-"),
            item.get("name").and_then(Value::as_str).unwrap_or("-"),
        ));
    }
    out
}

fn wanted_tracks(score: &Score, args: &Value) -> Result<Vec<usize>, String> {
    match args.get("tracks").and_then(Value::as_array) {
        Some(list) => {
            let picked: Vec<usize> =
                list.iter().filter_map(Value::as_u64).map(|n| n as usize).collect();
            if let Some(bad) = picked.iter().find(|i| **i >= score.tracks.len()) {
                return Err(format!(
                    "There is no track {bad} (tracks are 0–{}).",
                    score.tracks.len().saturating_sub(1)
                ));
            }
            Ok(picked)
        }
        None => Ok((0..score.tracks.len()).collect()),
    }
}

fn signature_summary(score: &Score, bars: &otomo_score::BarMap) -> String {
    let mut parts: Vec<String> = Vec::new();
    for span in &bars.spans {
        let label = format!("{}/{}", span.numerator, span.denominator);
        if span.first_bar == 1 {
            parts.push(label);
        } else {
            parts.push(format!("{label} (from bar {})", span.first_bar));
        }
        if parts.len() == 4 {
            parts.push("…".into());
            break;
        }
    }
    let _ = score;
    parts.join(" → ")
}

fn read(song: &Song, score: &Score, args: &Value) -> Result<String, String> {
    let bars = score.bars();
    let from = need_i64(args, "from_bar")?.max(1) as u32;
    let to = need_i64(args, "to_bar")?.max(from as i64) as u32;
    if from > bars.count {
        return Err(format!("This song ends at bar {} (bar {from} is out of range).", bars.count));
    }
    if to - from + 1 > MAX_BARS {
        return Err(format!(
            "At most {MAX_BARS} bars can be read at a time. Split it up, starting with bars {from}–{}.",
            from + MAX_BARS - 1
        ));
    }
    let velocity = args.get("velocity").and_then(Value::as_bool).unwrap_or(false);
    // Most files are performances; snapping is what makes them readable, and
    // the right grid depends on the passage, so it is chosen from the music.
    let chosen = opt_str(args, "quantize").map(|t| Grid::parse(&t)).transpose()?;

    let wanted = wanted_tracks(score, args)?;

    let to = to.min(bars.count);
    let grid = chosen.unwrap_or_else(|| suggest_grid(score, &bars, &wanted, from, to));
    let mut out = format!("{} — bars {from}–{to} (quantize: {})\n", song.title, grid.label());
    for index in wanted {
        let track = &score.tracks[index];
        let lines = render_bars(score, &bars, index, from, to, velocity, grid);
        if lines.iter().all(|l| l.notes.is_empty()) {
            out.push_str(&format!("\n[{index}] {} — rests throughout this range\n", track_label(track)));
            continue;
        }
        out.push_str(&format!("\n[{index}] {}\n", track_label(track)));
        for line in lines {
            let (num, den) = bars.time_signature(line.bar);
            let sig = if bars.spans.iter().any(|s| s.first_bar == line.bar) {
                format!(" ({num}/{den})")
            } else {
                String::new()
            };
            out.push_str(&format!("b{}{}  {}\n", line.bar, sig, line.notes));
        }
    }
    Ok(out)
}

fn track_label(track: &otomo_score::Track) -> String {
    let instrument =
        if track.is_percussion() { "Percussion" } else { program_name(track.program) };
    if track.name.is_empty() {
        instrument.to_string()
    } else {
        format!("{} · {instrument}", track.name)
    }
}

