use super::*;
use crate::text::suggest_grid;

/// Build a one-track SMF: 480 ppq, 4/4, a C major triad then a scale.
fn smf(ppq: u16, events: &[(u32, [u8; 3])], meta: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut track: Vec<(u32, Vec<u8>)> =
        events.iter().map(|(t, e)| (*t, e.to_vec())).chain(meta.iter().cloned()).collect();
    track.sort_by_key(|(t, _)| *t);

    let mut body = Vec::new();
    let mut last = 0u32;
    for (tick, bytes) in track {
        write_varlen(&mut body, tick - last);
        last = tick;
        body.extend_from_slice(&bytes);
    }
    write_varlen(&mut body, 0);
    body.extend_from_slice(&[0xff, 0x2f, 0x00]); // end of track

    let mut out = Vec::new();
    out.extend_from_slice(b"MThd");
    out.extend_from_slice(&6u32.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // format 0
    out.extend_from_slice(&1u16.to_be_bytes()); // one track
    out.extend_from_slice(&ppq.to_be_bytes());
    out.extend_from_slice(b"MTrk");
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

fn write_varlen(out: &mut Vec<u8>, mut value: u32) {
    let mut buf = vec![(value & 0x7f) as u8];
    value >>= 7;
    while value > 0 {
        buf.push(((value & 0x7f) as u8) | 0x80);
        value >>= 7;
    }
    buf.reverse();
    out.extend_from_slice(&buf);
}

fn meta(tick: u32, kind: u8, data: &[u8]) -> (u32, Vec<u8>) {
    let mut v = vec![0xff, kind, data.len() as u8];
    v.extend_from_slice(data);
    (tick, v)
}

#[test]
fn parses_notes_tempo_and_signatures() {
    const PPQ: u16 = 480;
    let bytes = smf(
        PPQ,
        &[
            // Bar 1: C major triad, a whole bar long.
            (0, [0x90, 60, 100]),
            (0, [0x90, 64, 90]),
            (0, [0x90, 67, 95]),
            (1920, [0x80, 60, 0]),
            (1920, [0x80, 64, 0]),
            (1920, [0x80, 67, 0]),
            // Bar 2: two quarter notes, the second released by a zero
            // velocity note-on (which is how many files write note-off).
            (1920, [0x90, 72, 80]),
            (2400, [0x80, 72, 0]),
            (2400, [0x90, 74, 80]),
            (2880, [0x90, 74, 0]),
        ],
        &[
            meta(0, 0x51, &[0x07, 0xa1, 0x20]), // 500000 us = 120 bpm
            meta(0, 0x58, &[4, 2, 24, 8]),      // 4/4
            meta(0, 0x59, &[2, 0]),             // D major
            meta(0, 0x03, b"Lead"),
            meta(1920, 0x51, &[0x09, 0x27, 0xc0]), // 600000 us = 100 bpm
        ],
    );

    let score = Score::parse(&bytes).expect("parse");
    assert_eq!(score.ppq, 480);
    assert_eq!(score.tracks.len(), 1);
    assert_eq!(score.tracks[0].name, "Lead");
    assert_eq!(score.tracks[0].notes.len(), 5);
    assert_eq!(score.key_signatures[0].name(), "D major");
    assert_eq!(score.bpm_range(), (100, 120));

    // Bar 1 at 120 bpm is 2 s; the notes after it run at 100 bpm.
    assert!((score.tick_seconds(1920) - 2.0).abs() < 1e-9, "{}", score.tick_seconds(1920));
    assert!((score.duration() - (2.0 + 0.6 + 0.6)).abs() < 1e-9, "{}", score.duration());

    let bars = score.bars();
    assert_eq!(bars.count, 2);
    assert_eq!(bars.bar_start(2), 1920);
    assert_eq!(bars.position(2400), (2, 2.0));

    let lines = render_bars(&score, &bars, 0, 1, 4, false, Grid::Raw);
    assert_eq!(lines.len(), 2, "bars past the end are skipped");
    assert_eq!(lines[0].notes, "[C4 E4 G4]@1:4");
    assert_eq!(lines[1].notes, "C5@1:1 D5@2:1");

    let with_vel = render_bars(&score, &bars, 0, 1, 1, true, Grid::Raw);
    assert_eq!(with_vel[0].notes, "[C4 E4 G4]@1:4v95");
}

#[test]
fn a_time_signature_change_renumbers_the_bars() {
    const PPQ: u16 = 480;
    // 4/4 for two bars, then 3/4; a note on the downbeat of each bar.
    let bytes = smf(
        PPQ,
        &[
            (0, [0x90, 60, 100]),
            (240, [0x80, 60, 0]),
            (3840, [0x90, 62, 100]),
            (4080, [0x80, 62, 0]),
            (5280, [0x90, 64, 100]), // 3840 + 1440 = bar 4's downbeat
            (5520, [0x80, 64, 0]),
        ],
        &[
            meta(0, 0x58, &[4, 2, 24, 8]),
            meta(3840, 0x58, &[3, 2, 24, 8]), // 3/4 from bar 3
        ],
    );
    let score = Score::parse(&bytes).expect("parse");
    let bars = score.bars();
    assert_eq!(bars.time_signature(2), (4, 4));
    assert_eq!(bars.time_signature(3), (3, 4));
    assert_eq!(bars.bar_start(3), 3840);
    assert_eq!(bars.bar_ticks(3), 1440);
    assert_eq!(bars.position(5280), (4, 1.0));

    let lines = render_bars(&score, &bars, 0, 1, bars.count, false, Grid::Raw);
    assert_eq!(lines[0].notes, "C4@1:0.5");
    assert_eq!(lines[1].notes, "", "empty bars render empty");
    assert_eq!(lines[2].notes, "D4@1:0.5");
    assert_eq!(lines[3].notes, "E4@1:0.5");
}

#[test]
fn a_file_without_tempo_or_signature_still_reads() {
    let bytes = smf(96, &[(0, [0x90, 60, 64]), (96, [0x80, 60, 0])], &[]);
    let score = Score::parse(&bytes).expect("parse");
    assert_eq!(score.bpm_range(), (120, 120));
    assert_eq!(score.bars().time_signature(1), (4, 4));
    assert_eq!(score.tracks[0].notes[0].len, 96);
}

#[test]
fn a_note_left_hanging_runs_to_the_end() {
    let bytes = smf(480, &[(0, [0x90, 60, 64]), (960, [0x90, 62, 64])], &[]);
    let score = Score::parse(&bytes).expect("parse");
    let notes = &score.tracks[0].notes;
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0].end(), 960, "closed by the end of the track");
}

#[test]
fn rejects_what_it_cannot_represent() {
    assert!(Score::parse(b"not a midi file").is_err());
}

#[test]
fn note_names_follow_middle_c_is_c4() {
    assert_eq!(note_name(60), "C4");
    assert_eq!(note_name(61), "C#4");
    assert_eq!(note_name(21), "A0");
    assert_eq!(note_name(108), "C8");
}

#[test]
fn snapping_turns_a_performance_back_into_a_score() {
    const PPQ: u16 = 480;
    // A rolled chord played slightly late and unevenly, as a human would.
    let bytes = smf(
        PPQ,
        &[
            (17, [0x90, 60, 100]),
            (33, [0x90, 64, 92]),
            (52, [0x90, 67, 88]),
            (470, [0x80, 60, 0]),
            (486, [0x80, 64, 0]),
            (501, [0x80, 67, 0]),
        ],
        &[meta(0, 0x58, &[4, 2, 24, 8])],
    );
    let score = Score::parse(&bytes).expect("parse");
    let bars = score.bars();

    let raw = render_bars(&score, &bars, 0, 1, 1, false, Grid::Raw);
    assert!(raw[0].notes.contains("@1.042"), "unsnapped keeps the timing: {}", raw[0].notes);

    let snapped = render_bars(&score, &bars, 0, 1, 1, false, Grid::Divisions(16));
    assert_eq!(snapped[0].notes, "[C4 E4 G4]@1:1", "one chord on the downbeat");
}

#[test]
fn snapping_never_pushes_a_note_past_the_bar_line() {
    const PPQ: u16 = 480;
    // A note 4 ticks before the end of bar 1: rounding would make it beat 5.
    let bytes = smf(
        PPQ,
        &[(1916, [0x90, 60, 100]), (2000, [0x80, 60, 0])],
        &[meta(0, 0x58, &[4, 2, 24, 8])],
    );
    let score = Score::parse(&bytes).expect("parse");
    let bars = score.bars();
    let lines = render_bars(&score, &bars, 0, 1, 1, false, Grid::Divisions(16));
    assert_eq!(lines[0].notes, "C4@4.75:0.25", "stays inside a 4/4 bar");
}

#[test]
fn grid_parsing_says_what_it_accepts() {
    assert_eq!(Grid::parse("1/16").unwrap(), Grid::Divisions(16));
    assert_eq!(Grid::parse(" off ").unwrap(), Grid::Raw);
    assert!(Grid::parse("sixteenth").is_err());
    assert!(Grid::parse("1/0").is_err());
    assert!(Grid::parse("1/128").is_err());
}

/* ---------- editing ---------- */

fn rendered(score: &Score, track: usize, from: u32, to: u32) -> Vec<String> {
    let bars = score.bars();
    render_bars(score, &bars, track, from, to, false, Grid::Divisions(16))
        .into_iter()
        .map(|l| l.notes)
        .collect()
}

#[test]
fn a_written_passage_survives_a_save_and_reload() {
    let mut score = Score::blank(96.0, 3, 4);
    score.add_track(40, "Violin", false);
    score.add_track(0, "Piano", false);
    score
        .write_bars(0, 1, 3, "b1  C5@1:1 D5@2:1 E5@3:1\nb2  F5@1:3\nb3  -\n")
        .expect("write");
    score
        .write_bars(1, 1, 2, "b1  [C3 E3 G3]@1:3\nb2  [F3 A3 C4]@1:3v96\n")
        .expect("write");
    score.set_key(-1, false).expect("key");

    let bytes = score.to_smf().expect("write smf");
    let reloaded = Score::parse(&bytes).expect("reparse");

    assert_eq!(reloaded.ppq, score.ppq);
    assert_eq!(reloaded.bpm_range(), (96, 96));
    assert_eq!(reloaded.bars().time_signature(1), (3, 4));
    assert_eq!(reloaded.key_signatures[0].name(), "F major");
    assert_eq!(reloaded.tracks.len(), 2);
    assert_eq!(reloaded.tracks[0].name, "Violin");
    assert_eq!(reloaded.tracks[0].program, 40);
    assert_eq!(reloaded.tracks[1].channel, 1, "each part gets its own channel");

    assert_eq!(rendered(&reloaded, 0, 1, 3), rendered(&score, 0, 1, 3));
    // The score ends where the music does, so the empty third bar is not
    // part of it — reading past the end simply stops.
    assert_eq!(rendered(&reloaded, 0, 1, 3), vec!["C5@1:1 D5@2:1 E5@3:1", "F5@1:3"]);
    assert_eq!(rendered(&reloaded, 1, 1, 2), vec!["[C3 E3 G3]@1:3", "[F3 A3 C4]@1:3"]);
}

#[test]
fn writing_the_same_range_twice_replaces_rather_than_piles_up() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(0, "Piano", false);
    score.write_bars(0, 1, 1, "C4@1:1 D4@2:1").expect("write");
    score.write_bars(0, 1, 1, "C4@1:1 D4@2:1").expect("write again");
    assert_eq!(score.tracks[0].notes.len(), 2, "idempotent");

    let summary = score.write_bars(0, 1, 1, "E4@1:4").expect("replace");
    assert_eq!(score.tracks[0].notes.len(), 1);
    assert!(summary.contains("2音 → 1音"), "{summary}");

    // Clearing a range leaves the rest alone.
    score.write_bars(0, 2, 2, "G4@1:4").expect("bar 2");
    score.write_bars(0, 1, 1, "").expect("clear bar 1");
    assert_eq!(rendered(&score, 0, 1, 2), vec!["", "G4@1:4"]);
}

#[test]
fn edits_refuse_what_they_cannot_do_and_say_why() {
    let mut score = Score::blank(120.0, 4, 4);
    assert!(score.write_bars(0, 1, 1, "C4@1:1").unwrap_err().contains("add_track"));
    score.add_track(0, "Piano", false);

    let err = score.write_bars(0, 1, 2, "b7  C4@1:1").unwrap_err();
    assert!(err.contains("7小節目") && err.contains("範囲"), "{err}");
    let err = score.write_bars(0, 1, 1, "C4@5:1").unwrap_err();
    assert!(err.contains("4/4") && err.contains("拍は"), "{err}");
    assert!(score.write_bars(0, 3, 1, "").unwrap_err().contains("from_bar 以上"));
    assert!(score.set_tempo(0.0, 1).unwrap_err().contains("10〜400"));
    assert!(score.set_time_signature(4, 5, 1).unwrap_err().contains("2の冪"));
    assert!(score.set_key(9, false).unwrap_err().contains("-7〜7"));
    assert!(score.remove_track(3).unwrap_err().contains("0〜0"));
}

#[test]
fn inserting_and_deleting_bars_moves_the_music() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(0, "Piano", false);
    score.write_bars(0, 1, 3, "b1  C4@1:4\nb2  D4@1:4\nb3  E4@1:4").expect("write");

    score.insert_bars(2, 1).expect("insert");
    assert_eq!(rendered(&score, 0, 1, 4), vec!["C4@1:4", "", "D4@1:4", "E4@1:4"]);

    score.delete_bars(2, 2).expect("delete");
    assert_eq!(rendered(&score, 0, 1, 3), vec!["C4@1:4", "D4@1:4", "E4@1:4"]);

    // A later tempo change travels with the music it belongs to.
    score.set_tempo(60.0, 3).expect("tempo");
    score.insert_bars(1, 1).expect("insert at the top");
    let bars = score.bars();
    assert_eq!(score.tempos.last().unwrap().tick, bars.bar_start(4));
}

#[test]
fn a_percussion_track_lands_on_channel_ten() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(0, "Drums", true);
    assert!(score.tracks[0].is_percussion());
    score.write_bars(0, 1, 1, "C2@1:0.25 F#2@1:0.25").expect("write");
    let reloaded = Score::parse(&score.to_smf().unwrap()).unwrap();
    assert!(reloaded.tracks[0].is_percussion(), "channel 10 survives the save");
}

#[test]
fn an_empty_track_survives_until_it_has_notes() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(48, "", false);
    assert_eq!(score.tracks[0].name, "Track 1", "an empty part must be named to survive");

    let reloaded = Score::parse(&score.to_smf().unwrap()).unwrap();
    assert_eq!(reloaded.tracks.len(), 1, "add_track then save then write_bars must work");
    assert_eq!(reloaded.tracks[0].program, 48);
}

#[test]
fn key_names_read_both_ways() {
    for sharps in -7i8..=7 {
        for minor in [false, true] {
            let name = KeyChange { tick: 0, sharps, minor }.name();
            assert_eq!(KeyChange::parse(&name).unwrap(), (sharps, minor), "{name}");
        }
    }
    assert_eq!(KeyChange::parse("D").unwrap(), (2, false), "a bare tonic is major");
    assert_eq!(KeyChange::parse("bb minor").unwrap(), KeyChange::parse("Bb minor").unwrap());
    assert_eq!(KeyChange::parse("B-flat major").unwrap(), (-2, false), "spelled out, as on the web");
    assert_eq!(KeyChange::parse("F sharp minor").unwrap(), (3, true));
    assert_eq!(KeyChange::parse("E-Flat").unwrap(), (-3, false));
    assert!(KeyChange::parse("H major").is_err());
    assert!(KeyChange::parse("C lydian").is_err());
}


#[test]
fn a_performance_keeps_its_pedal_and_expression_through_an_edit() {
    const PPQ: u16 = 480;
    let bytes = smf(
        PPQ,
        &[
            (0, [0xb0, 64, 127]), // sustain down
            (0, [0x90, 60, 90]),
            (0, [0xb0, 11, 100]), // expression
            (240, [0xe0, 0x00, 0x50]), // pitch bend
            (480, [0x80, 60, 0]),
            (480, [0xb0, 64, 0]), // sustain up
            (1920, [0x90, 62, 90]),
            (2400, [0x80, 62, 0]),
        ],
        &[
            meta(0, 0x58, &[4, 2, 24, 8]),
            meta(0, 0x03, b"Piano"),
            // An instrument switch partway through, which is not the program
            // the track starts on.
            (2400, vec![0xc0, 48]),
        ],
    );
    let score = Score::parse(&bytes).expect("parse");
    assert_eq!(score.tracks[0].program, 0, "a switch mid-piece is not the opening sound");
    assert_eq!(score.tracks[0].passthrough.len(), 5, "kept verbatim, note events aside");

    // Rewriting bar 1 must not cost the performance its shaping.
    let mut edited = score.clone();
    edited.write_bars(0, 1, 1, "E4@1:1").expect("write");
    let reloaded = Score::parse(&edited.to_smf().expect("save")).expect("reparse");

    let channel_events = |s: &Score| {
        let bytes = s.to_smf().unwrap();
        let smf = midly::Smf::parse(&bytes).unwrap();
        let (mut cc, mut bend, mut prog) = (0, 0, 0);
        for track in &smf.tracks {
            for e in track {
                if let midly::TrackEventKind::Midi { message, .. } = e.kind {
                    match message {
                        midly::MidiMessage::Controller { .. } => cc += 1,
                        midly::MidiMessage::PitchBend { .. } => bend += 1,
                        midly::MidiMessage::ProgramChange { .. } => prog += 1,
                        _ => {}
                    }
                }
            }
        }
        (cc, bend, prog)
    };
    // 3 controllers, 1 bend, and both programs (the opening one plus the switch).
    assert_eq!(channel_events(&score), (3, 1, 2));
    assert_eq!(channel_events(&reloaded), (3, 1, 2), "an edit must not flatten the file");
    assert_eq!(rendered(&reloaded, 0, 1, 2), vec!["E4@1:1", "D4@1:1"]);

    // Moving the music moves its pedalling with it.
    let mut shifted = score.clone();
    shifted.insert_bars(1, 1).expect("insert");
    assert!(
        shifted.tracks[0].passthrough.iter().all(|(tick, _)| *tick >= 1920),
        "controllers travel with the bars they shape"
    );
}


#[test]
fn orchestrating_is_copying_a_part_around() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(0, "Piano", false);
    score.add_track(40, "Violin", false);
    score.add_track(42, "Cello", false);
    // A piano texture: melody on top, bass underneath.
    score
        .write_bars(0, 1, 2, "b1  [C3 E4 G4 C5]@1:4\nb2  [F3 A4 C5 F5]@1:4")
        .expect("write");

    // The violins take the melody, an octave up.
    score.copy_bars(0, 1, 1, 2, 1, 12, Voice::Top, false).expect("melody");
    assert_eq!(rendered(&score, 1, 1, 2), vec!["C6@1:4", "F6@1:4"]);

    // The cellos take the bass line where it stands.
    let summary = score.copy_bars(0, 2, 1, 2, 1, 0, Voice::Bottom, false).expect("bass");
    assert_eq!(rendered(&score, 2, 1, 2), vec!["C3@1:4", "F3@1:4"]);
    assert!(summary.contains("Cello") && summary.contains("2音"), "{summary}");

    // Copying a passage later in the piece leaves the original alone.
    score.copy_bars(0, 0, 1, 2, 3, 0, Voice::All, false).expect("repeat");
    assert_eq!(
        rendered(&score, 0, 1, 4),
        vec!["[C3 E4 G4 C5]@1:4", "[F3 A4 C5 F5]@1:4", "[C3 E4 G4 C5]@1:4", "[F3 A4 C5 F5]@1:4"]
    );

    // Merging doubles a part instead of replacing it.
    score.copy_bars(0, 1, 1, 1, 1, 0, Voice::Bottom, true).expect("double");
    assert_eq!(rendered(&score, 1, 1, 1), vec!["[C3 C6]@1:4"], "an octave doubling");

    assert!(score.copy_bars(0, 1, 1, 2, 1, 96, Voice::Top, false).unwrap_err().contains("音域"));
    assert!(score.copy_bars(1, 2, 90, 92, 1, 0, Voice::All, false).unwrap_err().contains("音がありません"));
}

#[test]
fn a_whole_page_of_score_goes_in_one_call() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(73, "Flute", false);
    score.add_track(40, "Violin", false);
    score.add_track(42, "Cello", false);

    let summary = score
        .write_score(
            1,
            2,
            "[0] Flute\nb1  C6@1:2 D6@3:2\nb2  E6@1:4\n\n[2] Cello\nb1  C3@1:4\nb2  G2@1:4\n",
        )
        .expect("write score");
    assert_eq!(summary.lines().count(), 2, "one line per part touched");
    assert_eq!(rendered(&score, 0, 1, 2), vec!["C6@1:2 D6@3:2", "E6@1:4"]);
    assert_eq!(rendered(&score, 2, 1, 2), vec!["C3@1:4", "G2@1:4"]);
    assert_eq!(rendered(&score, 1, 1, 2), vec!["", ""], "a part left out is left alone");

    // What read_bars prints is what write_score accepts.
    let bars = score.bars();
    let mut page = String::new();
    for track in [0usize, 2] {
        page.push_str(&format!("[{track}] {}\n", score.tracks[track].display_name()));
        for line in render_bars(&score, &bars, track, 1, 2, false, Grid::Divisions(16)) {
            page.push_str(&format!("b{}  {}\n", line.bar, line.notes));
        }
    }
    let mut copy = Score::blank(120.0, 4, 4);
    copy.add_track(73, "Flute", false);
    copy.add_track(40, "Violin", false);
    copy.add_track(42, "Cello", false);
    copy.write_score(1, 2, &page).expect("round trip");
    assert_eq!(rendered(&copy, 0, 1, 2), rendered(&score, 0, 1, 2));
    assert_eq!(rendered(&copy, 2, 1, 2), rendered(&score, 2, 1, 2));

    assert!(score.write_score(1, 2, "C4@1:1").unwrap_err().contains("見出し"));
    assert!(score.write_score(1, 2, "[9] Nope\nC4@1:1").unwrap_err().contains("0〜2"));
}

#[test]
fn a_drum_part_is_written_in_drum_names() {
    let mut score = Score::blank(100.0, 4, 4);
    score.add_track(0, "Kit", true);
    score
        .write_bars(0, 1, 1, "Kick@1:0.25 HatClosed@1:0.25 Snare@2:0.25 HatClosed@2:0.25")
        .expect("write");
    assert_eq!(
        rendered(&score, 0, 1, 1),
        vec!["[Kick HatClosed]@1:0.25 [Snare HatClosed]@2:0.25"]
    );
    // Note names still work, and mean the same thing.
    let mut same = Score::blank(100.0, 4, 4);
    same.add_track(0, "Kit", true);
    same.write_bars(0, 1, 1, "C2@1:0.25 F#2@1:0.25").expect("write");
    assert_eq!(same.tracks[0].notes[0].key, 36);
    assert!(same.write_bars(0, 1, 1, "Kik@1:1").unwrap_err().contains("Kick"));
}

#[test]
fn an_empty_drum_track_keeps_its_channel_across_a_save() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(0, "Drums", true);
    score.add_track(73, "Flute", false);
    let reloaded = Score::parse(&score.to_smf().unwrap()).unwrap();
    assert!(reloaded.tracks[0].is_percussion(), "a kit with no notes is still a kit");
    assert!(!reloaded.tracks[1].is_percussion());
    // And so a drum part written after the save still reads as drums.
    let mut reloaded = reloaded;
    reloaded.write_bars(0, 1, 1, "Kick@1:1 Snare@3:1").expect("write");
    assert_eq!(rendered(&reloaded, 0, 1, 1), vec!["Kick@1:1 Snare@3:1"]);
}

#[test]
fn notes_are_spelled_the_way_the_key_signature_does() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(0, "Piano", false);
    score.write_bars(0, 1, 1, "Bb3@1:2 F#4@3:2").expect("write");
    assert_eq!(rendered(&score, 0, 1, 1), vec!["A#3@1:2 F#4@3:2"], "sharps by default");

    score.set_key(-1, false).expect("F major");
    assert_eq!(rendered(&score, 0, 1, 1), vec!["Bb3@1:2 Gb4@3:2"], "flats in a flat key");

    score.set_key(2, false).expect("D major");
    assert_eq!(rendered(&score, 0, 1, 1), vec!["A#3@1:2 F#4@3:2"]);
}

#[test]
fn the_chosen_grid_is_fine_enough_for_the_notes_themselves() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(0, "Kit", true);
    // Eighth notes on every beat: the onsets sit on quarters, but printing
    // them as quarters would double every note's length.
    score
        .write_bars(0, 1, 1, "Kick@1:0.5 HatClosed@2:0.5 Snare@3:0.5 HatClosed@4:0.5")
        .expect("write");
    let bars = score.bars();
    assert_eq!(suggest_grid(&score, &bars, &[0], 1, 1), Grid::Divisions(8));

    let mut whole = Score::blank(120.0, 4, 4);
    whole.add_track(0, "Choir", false);
    whole.write_bars(0, 1, 2, "b1  [C4 E4 G4]@1:4\nb2  [F4 A4 C5]@1:4").expect("write");
    let bars = whole.bars();
    assert_eq!(suggest_grid(&whole, &bars, &[0], 1, 2), Grid::Divisions(4));
}

#[test]
fn the_fastest_part_on_the_page_sets_the_grid() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(48, "Strings", false);
    score.add_track(0, "Kit", true);
    score.write_bars(0, 1, 1, "[C3 G3]@1:4").expect("held chord");
    score
        .write_bars(1, 1, 1, "Kick@1:0.5 HatClosed@2:0.5 Snare@3:0.5 HatClosed@4:0.5")
        .expect("eighths");
    let bars = score.bars();
    assert_eq!(
        suggest_grid(&score, &bars, &[0, 1], 1, 1),
        Grid::Divisions(8),
        "the strings' whole notes must not flatten the kit"
    );
    assert_eq!(
        render_bars(&score, &bars, 1, 1, 1, false, Grid::Divisions(8))[0].notes,
        "Kick@1:0.5 HatClosed@2:0.5 Snare@3:0.5 HatClosed@4:0.5"
    );
}

#[test]
fn a_pickup_is_written_as_a_short_first_bar() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(73, "Flute", false);
    // An upbeat of one quarter, then two full bars.
    score.write_bars(0, 1, 3, "b1  G4@4:1\nb2  C5@1:2 E5@3:2\nb3  G5@1:4").expect("write");

    assert_eq!(score.looks_like_a_pickup(), Some(1.0), "the upbeat is there but undeclared");
    score.set_pickup(1.0).expect("pickup");
    assert_eq!(score.looks_like_a_pickup(), None, "and now it is declared");
    let bars = score.bars();
    assert_eq!(bars.time_signature(1), (1, 4), "bar 1 holds just the upbeat");
    assert_eq!(bars.time_signature(2), (4, 4));
    assert_eq!(bars.bar_start(2), 480);

    // It survives a save, because it is only a time signature.
    let reloaded = Score::parse(&score.to_smf().unwrap()).unwrap();
    assert_eq!(reloaded.bars().time_signature(1), (1, 4));
    assert_eq!(reloaded.bars().time_signature(2), (4, 4));

    assert!(score.set_pickup(4.0).unwrap_err().contains("短く"));
}

#[test]
fn an_undeclared_upbeat_is_noticed() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(73, "Flute", false);
    score.write_bars(0, 1, 2, "b1  G4@4:1\nb2  C5@1:4").expect("write");
    assert_eq!(score.looks_like_a_pickup(), Some(1.0), "the music starts on beat 4");

    let mut square = Score::blank(120.0, 4, 4);
    square.add_track(73, "Flute", false);
    square.write_bars(0, 1, 1, "C5@1:4").expect("write");
    assert_eq!(square.looks_like_a_pickup(), None);
}

#[test]
fn deleting_bars_does_not_pile_the_pedal_on_the_seam() {
    const PPQ: u16 = 480;
    let bytes = smf(
        PPQ,
        &[
            (0, [0x90, 60, 90]),
            (480, [0x80, 60, 0]),
            // Bar 2: a pedal gesture and a volume ramp that will be cut.
            (1920, [0xb0, 64, 127]),
            (2000, [0xb0, 7, 100]),
            (2100, [0xb0, 7, 110]),
            (2200, [0xb0, 7, 120]),
            (3800, [0xb0, 64, 0]),
            (3840, [0x90, 62, 90]),
            (4320, [0x80, 62, 0]),
        ],
        &[meta(0, 0x58, &[4, 2, 24, 8]), meta(0, 0x03, b"Piano")],
    );
    let mut score = Score::parse(&bytes).expect("parse");
    score.delete_bars(2, 2).expect("delete");

    let kept = &score.tracks[0].passthrough;
    // One last value per controller, at the cut — not the whole passage.
    assert_eq!(kept.len(), 2, "only the latched values survive: {kept:?}");
    assert!(kept.iter().all(|(tick, _)| *tick == 1920), "and they sit on the seam");
    assert_eq!(rendered(&score, 0, 1, 2), vec!["C4@1:1", "D4@1:1"]);
}

#[test]
fn dynamics_shape_a_passage_and_a_swell_draws_a_curve() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(48, "Strings", false);
    score.write_bars(0, 1, 4, "b1  C4@1:4\nb2  D4@1:4\nb3  E4@1:4\nb4  F4@1:4").expect("write");

    score.set_dynamics(0, 1, 4, "p", None).expect("flat");
    assert!(score.tracks[0].notes.iter().all(|n| n.velocity == 48), "one level throughout");
    assert!(score.tracks[0].passthrough.is_empty(), "no curve where nothing changes");

    let summary = score.set_dynamics(0, 1, 4, "p", Some("ff")).expect("crescendo");
    assert!(summary.contains("クレッシェンド"), "{summary}");
    let velocities: Vec<u8> = score.tracks[0].notes.iter().map(|n| n.velocity).collect();
    assert_eq!(velocities[0], 48);
    assert!(velocities.windows(2).all(|w| w[0] < w[1]), "rising: {velocities:?}");
    // Held notes cannot swell from velocity alone, so expression is written.
    assert!(
        score.tracks[0].passthrough.len() >= 8,
        "an expression curve: {}",
        score.tracks[0].passthrough.len()
    );

    // Re-shaping replaces the old curve rather than layering a second one.
    let before = score.tracks[0].passthrough.len();
    score.set_dynamics(0, 1, 4, "ff", Some("p")).expect("diminuendo");
    assert_eq!(score.tracks[0].passthrough.len(), before, "one curve, not two");

    assert!(score.set_dynamics(0, 1, 4, "loud", None).unwrap_err().contains("pp"));
    assert!(score.set_dynamics(0, 9, 9, "f", None).unwrap_err().contains("音がありません"));
}

#[test]
fn a_ritardando_is_a_curve_not_a_step() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(48, "Strings", false);
    score.write_bars(0, 1, 4, "b1  C4@1:4\nb2  D4@1:4\nb3  E4@1:4\nb4  F4@1:4").expect("write");

    let summary = score.tempo_curve(3, 4, 120.0, 60.0).expect("rit");
    assert!(summary.contains("rit."), "{summary}");
    let bpms: Vec<u32> = score.tempos.iter().map(|t| t.bpm().round() as u32).collect();
    assert!(bpms.len() >= 8, "sampled along the way: {bpms:?}");
    assert_eq!(*bpms.last().unwrap(), 60);
    assert!(bpms.windows(2).all(|w| w[0] >= w[1]), "never speeds up: {bpms:?}");
    // And it survives being written out.
    let reloaded = Score::parse(&score.to_smf().unwrap()).unwrap();
    assert_eq!(reloaded.bpm_range(), (60, 120));
}
