use super::rhythm::{BarContext, Meter, Piece};
use super::*;
use crate::{read_musicxml, Note, Score};

/* ---------- note values ---------- */

fn spell(meter: (u8, u16), o: u32, e: u32, rest: bool) -> Vec<(u32, u32, &'static str, u8)> {
    spell_with(meter, &[], o, e, rest)
}

fn spell_with(meter: (u8, u16), triplets: &[u32], o: u32, e: u32, rest: bool) -> Vec<(u32, u32, &'static str, u8)> {
    let is_triplet = |r: u32| triplets.contains(&r);
    let ctx = BarContext { meter: Meter { num: meter.0, den: meter.1 }, shift: 0, triplets: &is_triplet };
    ctx.spell(o, e, rest)
        .into_iter()
        .map(|Piece { offset, len, value }| (offset, len, value.kind.name(), value.dots))
        .collect()
}

const Q: u32 = DIV;

#[test]
fn notes_show_the_beat() {
    // A whole bar, and a dotted half from the downbeat: one note each.
    assert_eq!(spell((4, 4), 0, 4 * Q, false), [(0, 4 * Q, "whole", 0)]);
    assert_eq!(spell((4, 4), 0, 3 * Q, false), [(0, 3 * Q, "half", 1)]);
    // Quarter–half–quarter is written as it is played...
    assert_eq!(spell((4, 4), Q, 3 * Q, false), [(Q, 2 * Q, "half", 0)]);
    // ...but a dotted half from beat two shows beat three.
    assert_eq!(spell((4, 4), Q, 4 * Q, false), [(Q, Q, "quarter", 0), (2 * Q, 2 * Q, "half", 0)]);
    // Eighth–quarter–eighth.
    assert_eq!(spell((4, 4), Q / 2, 3 * Q / 2, false), [(Q / 2, Q, "quarter", 0)]);
    // Off the beat and across it: tied at the beat.
    assert_eq!(spell((4, 4), Q / 2, 2 * Q, false), [(Q / 2, Q / 2, "eighth", 0), (Q, Q, "quarter", 0)]);
    // Within a beat anything goes: a sixteenth and a dotted eighth.
    assert_eq!(spell((4, 4), Q / 4, Q, false), [(Q / 4, 3 * Q / 4, "eighth", 1)]);
    // 3/4: a half on beat two is fine.
    assert_eq!(spell((3, 4), Q, 3 * Q, false), [(Q, 2 * Q, "half", 0)]);
    // 6/8: across the middle of the bar is never one note.
    assert_eq!(spell((6, 8), Q, 2 * Q, false), [(Q, Q / 2, "eighth", 0), (3 * Q / 2, Q / 2, "eighth", 0)]);
    assert_eq!(spell((6, 8), 0, 3 * Q / 2, false), [(0, 3 * Q / 2, "quarter", 1)]);
}

#[test]
fn rests_are_stricter_than_notes() {
    // A rest from beat two to the end shows beat three.
    assert_eq!(spell((4, 4), Q, 4 * Q, true), [(Q, Q, "quarter", 0), (2 * Q, 2 * Q, "half", 0)]);
    // And never syncopates.
    assert_eq!(spell((4, 4), Q, 3 * Q, true), [(Q, Q, "quarter", 0), (2 * Q, Q, "quarter", 0)]);
    assert_eq!(spell((3, 4), Q, 3 * Q, true), [(Q, Q, "quarter", 0), (2 * Q, Q, "quarter", 0)]);
}

#[test]
fn triplets_stay_inside_their_beat() {
    let third = Q / 3;
    assert_eq!(
        spell_with((4, 4), &[0], 0, Q, false),
        [(0, Q, "quarter", 0)],
        "a whole triplet group is just a quarter"
    );
    assert_eq!(spell_with((4, 4), &[0], third, Q, false), [(third, 2 * third, "quarter", 0)]);
    // Across into a plain beat: tied at the edge of the group.
    assert_eq!(
        spell_with((4, 4), &[0], 2 * third, 2 * Q, false),
        [(2 * third, third, "eighth", 0), (Q, Q, "quarter", 0)]
    );
}

/* ---------- whole songs, through MusicXML and back ---------- */

fn options(tracks: &[usize]) -> ExportOptions {
    ExportOptions { title: "Test".into(), tracks: tracks.to_vec(), transposed: true, multi_rests: false, max_bars: None }
}

fn notes(score: &Score, track: usize) -> Vec<(u32, u32, u8)> {
    let mut out: Vec<(u32, u32, u8)> = score.tracks[track].notes.iter().map(|n: &Note| (n.start, n.len, n.key)).collect();
    out.sort_unstable();
    out
}

/// Write the score, check it is well-formed XML, read it back.
fn round_trip(score: &Score, opts: &ExportOptions) -> (String, Score) {
    let xml = to_musicxml(score, opts).expect("export");
    roxmltree::Document::parse_with_options(
        &xml,
        roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() },
    )
    .unwrap_or_else(|e| panic!("not well-formed: {e}\n{xml}"));
    let read = read_musicxml(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    (xml, read.score)
}

#[test]
fn a_melody_comes_back_note_for_note() {
    let mut score = Score::blank(96.0, 4, 4);
    score.add_track(73, "Flute", false);
    score
        .write_bars(0, 1, 3, "b1  C5@1:1 D5@2:0.5 E5@2.5:0.5 F5@3:2\nb2  G5@1:1.5 A5@2.5:0.5 B5@3:1 C6@4:2\nb3  C6@2:1")
        .unwrap();
    let (xml, back) = round_trip(&score, &options(&[0]));
    assert_eq!(notes(&back, 0), notes(&score, 0), "{xml}");
    assert!(xml.contains("<work-title>Test</work-title>"));
    assert!(xml.contains("<per-minute>96</per-minute>"));
    // The C6 from beat four is tied over the bar line.
    assert!(xml.contains(r#"<tie type="start"/>"#) && xml.contains(r#"<tie type="stop"/>"#));
    // Eighths are beamed.
    assert!(xml.contains(r#"<beam number="1">begin</beam>"#));
    assert_eq!(back.tracks[0].program, 73);
}

#[test]
fn played_timing_is_tidied_onto_the_grid() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(0, "Piano", false);
    // Quarters played a little early and late, held a little short.
    let played = [(0u32, 430u32, 60u8), (497, 440, 62), (950, 470, 64), (1452, 410, 65)];
    score.tracks[0].notes = played.iter().map(|&(start, len, key)| Note { start, len, key, velocity: 80 }).collect();
    score.end_tick = 1920;
    let (xml, back) = round_trip(&score, &options(&[0]));
    assert_eq!(
        notes(&back, 0),
        [(0, 480, 60), (480, 480, 62), (960, 480, 64), (1440, 480, 65)],
        "{xml}"
    );
}

#[test]
fn triplets_are_written_as_triplets() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(40, "Violin", false);
    // Triplet eighths on beat one, then a quarter, then a triplet quarter + eighth.
    let t = 160u32; // a triplet eighth at 480 ppq
    score.tracks[0].notes = vec![
        Note { start: 0, len: t, key: 72, velocity: 80 },
        Note { start: t, len: t, key: 74, velocity: 80 },
        Note { start: 2 * t, len: t, key: 76, velocity: 80 },
        Note { start: 480, len: 480, key: 77, velocity: 80 },
        Note { start: 960, len: 2 * t, key: 79, velocity: 80 },
        Note { start: 960 + 2 * t, len: t, key: 81, velocity: 80 },
    ];
    score.end_tick = 1920;
    let (xml, back) = round_trip(&score, &options(&[0]));
    assert_eq!(notes(&back, 0), notes(&score, 0), "{xml}");
    assert_eq!(xml.matches("<actual-notes>3</actual-notes>").count(), 5, "{xml}");
    assert_eq!(xml.matches(r#"<tuplet type="start"/>"#).count(), 2);
}

#[test]
fn a_clarinet_reads_a_tone_up_and_sounds_where_it_did() {
    let mut score = Score::blank(120.0, 4, 4);
    score.set_key(-1, false).unwrap(); // F major
    score.add_track(71, "Clarinet", false);
    score.write_bars(0, 1, 1, "F4@1:1 A4@2:1 Bb4@3:1 C5@4:1").unwrap();

    let (xml, back) = round_trip(&score, &options(&[0]));
    assert_eq!(notes(&back, 0), notes(&score, 0), "sounding pitches survive:\n{xml}");
    assert!(xml.contains("<part-name>Clarinet in B♭</part-name>"), "{xml}");
    assert!(xml.contains("<fifths>1</fifths>"), "F major reads as G major:\n{xml}");
    assert!(xml.contains("<chromatic>-2</chromatic>"));
    // The written B♭ is a C: no accidentals at all in G major.
    assert!(!xml.contains("<accidental>"), "{xml}");

    // At concert pitch: as it sounds, in the concert key.
    let concert = ExportOptions { transposed: false, ..options(&[0]) };
    let (xml, back) = round_trip(&score, &concert);
    assert_eq!(notes(&back, 0), notes(&score, 0));
    assert!(xml.contains("<fifths>-1</fifths>") && !xml.contains("<transpose>"), "{xml}");
    assert!(xml.contains("<part-name>Clarinet</part-name>"));
}

#[test]
fn accidentals_follow_the_bar() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(73, "Flute", false);
    score.write_bars(0, 1, 2, "b1  F#5@1:1 F#5@2:1 F5@3:1 F#5@4:1\nb2  F#5@1:4").unwrap();
    let xml = to_musicxml(&score, &options(&[0])).unwrap();
    let marks: Vec<&str> = xml
        .lines()
        .filter_map(|l| l.trim().strip_prefix("<accidental>").and_then(|r| r.strip_suffix("</accidental>")))
        .collect();
    assert_eq!(marks, ["sharp", "natural", "sharp", "sharp"], "{xml}");
}

#[test]
fn a_piano_gets_two_staves_and_overlaps_become_voices() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(0, "Piano", false);
    // Right hand: a held half note under moving quarters. Left hand: chords.
    score
        .write_bars(0, 1, 1, "C5@1:2 E5@1:1 F5@2:1 [G5 C6]@3:2 [C3 G3]@1:2 [F2 C3]@3:2")
        .unwrap();
    let (xml, back) = round_trip(&score, &options(&[0]));
    assert_eq!(notes(&back, 0), notes(&score, 0), "{xml}");
    assert!(xml.contains("<staves>2</staves>"));
    assert!(xml.contains("<voice>2</voice>"), "the held C5 is a second voice:\n{xml}");
    assert!(xml.contains("<voice>5</voice>"), "the left hand is on staff 2:\n{xml}");
    assert!(xml.contains("<stem>down</stem>"));
}

#[test]
fn a_wind_part_plays_legato_rather_than_in_two_voices() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(68, "Oboe", false);
    // Each note held a sixteenth into the next (the last one ends with the bar).
    score.tracks[0].notes = (0..4)
        .map(|i| Note { start: i * 480, len: if i == 3 { 480 } else { 600 }, key: 72 + i as u8, velocity: 80 })
        .collect();
    score.end_tick = 1920;
    let xml = to_musicxml(&score, &options(&[0])).unwrap();
    assert!(!xml.contains("<voice>2</voice>"), "{xml}");
    assert_eq!(xml.matches("<type>quarter</type>").count(), 4, "{xml}");
}

#[test]
fn drums_are_written_on_a_percussion_staff() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(0, "Drums", true);
    score
        .write_bars(0, 1, 1, "[Kick HatClosed]@1:0.25 HatClosed@1.5:0.25 [Snare HatClosed]@2:0.25 HatClosed@2.5:0.25 [Kick HatClosed]@3:0.25 HatClosed@3.5:0.25 [Snare HatClosed]@4:0.25 HatClosed@4.5:0.25")
        .unwrap();
    let (xml, back) = round_trip(&score, &options(&[0]));
    assert!(back.tracks[0].is_percussion(), "{xml}");
    let mut keys: Vec<u8> = back.tracks[0].notes.iter().map(|n| n.key).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys, [36, 38, 42]);
    assert!(xml.contains("<sign>percussion</sign>"));
    assert!(xml.contains("<notehead>x</notehead>"));
    assert!(xml.contains("<midi-unpitched>37</midi-unpitched>"), "kick is key 36, 1-based in MusicXML");
    // Hats and snare stems up, kick down, in eighths and quarters.
    assert!(xml.contains("<stem>up</stem>") && xml.contains("<stem>down</stem>"));
    assert!(xml.contains("<type>eighth</type>"));
}

#[test]
fn a_pickup_is_bar_zero() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(73, "Flute", false);
    score.write_bars(0, 1, 3, "b1  G4@4:1\nb2  C5@1:2 E5@3:2\nb3  G5@1:4").unwrap();
    score.set_pickup(1.0).unwrap();
    let (xml, back) = round_trip(&score, &options(&[0]));
    assert!(xml.contains(r#"<measure number="0" implicit="yes">"#), "{xml}");
    assert!(xml.contains("<beats>4</beats>") && !xml.contains("<beats>1</beats>"), "printed in 4/4:\n{xml}");
    assert_eq!(notes(&back, 0), notes(&score, 0));
}

#[test]
fn a_part_folds_its_empty_bars_into_one_rest() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(56, "Trumpet", false);
    score.add_track(57, "Trombone", false);
    score.write_bars(0, 1, 7, "b1  C5@1:4\nb7  D5@1:4").unwrap();
    score.write_bars(1, 1, 7, "b1  C3@1:4\nb2  C3@1:4\nb3  C3@1:4\nb4  C3@1:4\nb5  C3@1:4\nb6  C3@1:4\nb7  C3@1:4").unwrap();
    let part = ExportOptions { multi_rests: true, ..options(&[0]) };
    let (xml, back) = round_trip(&score, &part);
    assert!(xml.contains("<multiple-rest>5</multiple-rest>"), "{xml}");
    assert_eq!(notes(&back, 0), notes(&score, 0));
    // The full score keeps every bar.
    let xml = to_musicxml(&score, &options(&[0, 1])).unwrap();
    assert!(!xml.contains("multiple-rest"));
    assert!(xml.contains(r#"<part-group type="start" number="1">"#), "two brass share a bracket");
}

#[test]
fn compound_and_odd_meters_survive() {
    for (num, den, text) in [
        (6u8, 8u16, "b1  C5@1:1.5 D5@2.5:0.5 E5@3:1 F5@4:1.5 G5@5.5:0.5 A5@6:1\nb2  C6@1:3"),
        (3, 4, "b1  C5@1:1 D5@2:2\nb2  E5@1:3"),
        (5, 4, "b1  C5@1:2 D5@3:3"),
        (7, 8, "b1  C5@1:1 D5@2:1 E5@3:1 F5@4:2 G5@6:2"),
        (2, 2, "b1  C5@1:0.5 D5@1.5:1 E5@2.5:0.5\nb2  F5@1:2"),
    ] {
        let mut score = Score::blank(100.0, num, den);
        score.add_track(73, "Flute", false);
        score.write_bars(0, 1, 2, text).unwrap();
        let (xml, back) = round_trip(&score, &options(&[0]));
        assert_eq!(notes(&back, 0), notes(&score, 0), "{num}/{den}:\n{xml}");
    }
}

#[test]
fn every_value_written_is_a_real_note_value() {
    // A messy performance: whatever it becomes, durations add up per voice
    // and every note has a type.
    let mut score = Score::blank(133.0, 4, 4);
    score.add_track(0, "Piano", false);
    let mut seed = 7u32;
    let mut rand = |n: u32| {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        (seed >> 16) % n
    };
    let mut ns = Vec::new();
    for _ in 0..200 {
        ns.push(Note { start: rand(16 * 1920), len: 30 + rand(1500), key: 36 + rand(48) as u8, velocity: 40 + rand(80) as u8 });
    }
    ns.sort_by_key(|n| (n.start, n.key));
    score.end_tick = ns.iter().map(|n| n.end()).max().unwrap();
    score.tracks[0].notes = ns;
    let xml = to_musicxml(&score, &options(&[0])).unwrap();
    let doc = roxmltree::Document::parse_with_options(&xml, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() })
        .unwrap();
    for measure in doc.descendants().filter(|n| n.has_tag_name("measure")) {
        let mut per_voice: std::collections::HashMap<String, u32> = Default::default();
        for note in measure.children().filter(|n| n.has_tag_name("note")) {
            if note.children().any(|c| c.has_tag_name("chord")) {
                continue;
            }
            let voice = note.children().find(|c| c.has_tag_name("voice")).unwrap().text().unwrap().to_string();
            let d: u32 = note.children().find(|c| c.has_tag_name("duration")).unwrap().text().unwrap().parse().unwrap();
            *per_voice.entry(voice).or_default() += d;
            let measure_rest = note.children().any(|c| c.has_tag_name("rest") && c.attribute("measure") == Some("yes"));
            assert!(measure_rest || note.children().any(|c| c.has_tag_name("type")), "a note without a type");
        }
        for (voice, total) in per_voice {
            assert_eq!(total, 4 * DIV, "bar {} voice {voice}", measure.attribute("number").unwrap());
        }
    }
}

#[test]
fn bar_one_never_disappears_into_a_multi_bar_rest() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(60, "Horn", false);
    score.write_bars(0, 1, 6, "b6  C4@1:4").unwrap();
    let part = ExportOptions { multi_rests: true, ..options(&[0]) };
    let xml = to_musicxml(&score, &part).unwrap();
    // Bar 1 on its own, bars 2-5 as one rest.
    assert!(xml.contains("<multiple-rest>4</multiple-rest>"), "{xml}");
    let first = xml.split(r#"<measure number="2">"#).next().unwrap();
    assert!(!first.contains("multiple-rest"));
}

#[test]
fn short_stabs_are_written_staccato() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(81, "Lead", false);
    // Sixteenths every eighth: a stab and a gap.
    score.write_bars(0, 1, 1, "C5@1:0.25 C5@1.5:0.25 D5@2:0.25 D5@2.5:0.25 E5@3:1 F5@4:0.25").unwrap();
    let xml = to_musicxml(&score, &options(&[0])).unwrap();
    assert_eq!(xml.matches("<staccato/>").count(), 5, "{xml}");
    assert_eq!(xml.matches("<type>eighth</type>").count(), 4, "{xml}");
    // The last one has the rest of its beat: a staccato quarter.
    assert!(!xml.contains("<type>16th</type>"), "{xml}");
}

#[test]
fn a_preview_stops_after_its_bars() {
    let mut score = Score::blank(120.0, 4, 4);
    score.add_track(73, "Flute", false);
    score.write_bars(0, 1, 4, "b1  C5@1:4\nb2  C5@1:4\nb3  C5@1:4\nb4  C5@1:4").unwrap();
    let preview = ExportOptions { max_bars: Some(2), ..options(&[0]) };
    let xml = to_musicxml(&score, &preview).unwrap();
    assert!(xml.contains(r#"<measure number="2">"#) && !xml.contains(r#"<measure number="3">"#));
}
