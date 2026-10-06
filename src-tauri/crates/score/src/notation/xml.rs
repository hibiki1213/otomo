//! The sheet as MusicXML 4.0 (partwise), element order as the schema wants
//! it so strict readers take it too.

use std::fmt::Write as _;

use super::instrument::Family;
use super::{Bar, HeadPitch, Item, PartBar, PartSheet, Sheet, VoiceBar};
use crate::gm;

pub(crate) fn write(sheet: &Sheet) -> String {
    let mut x = Xml::default();
    x.line(r#"<?xml version="1.0" encoding="UTF-8" standalone="no"?>"#);
    x.line(r#"<!DOCTYPE score-partwise PUBLIC "-//Recordare//DTD MusicXML 4.0 Partwise//EN" "http://www.musicxml.org/dtds/partwise.dtd">"#);
    x.open("score-partwise", &[("version", "4.0")]);
    if !sheet.title.is_empty() {
        x.open("work", &[]);
        x.leaf("work-title", &sheet.title);
        x.close("work");
    }
    x.open("identification", &[]);
    x.open("encoding", &[]);
    x.leaf("software", "Otomo");
    // What this file states outright, so readers do not redo it.
    for element in ["accidental", "beam", "stem"] {
        x.empty("supports", &[("element", element), ("type", "yes")]);
    }
    x.close("encoding");
    x.close("identification");

    part_list(&mut x, &sheet.parts);
    for (p, part) in sheet.parts.iter().enumerate() {
        x.open("part", &[("id", &part_id(p))]);
        for (b, bar) in sheet.bars.iter().enumerate() {
            measure(&mut x, part, p, bar, &part.bars[b], b == 0);
        }
        x.close("part");
    }
    x.close("score-partwise");
    x.out
}

fn part_id(p: usize) -> String {
    format!("P{}", p + 1)
}

/// Instrument ids: one for a pitched part, one per drum for a kit.
fn instrument_id(p: usize, key: Option<u8>) -> String {
    format!("P{}-I{}", p + 1, key.map_or(1, |k| u32::from(k) + 1))
}

fn part_list(x: &mut Xml, parts: &[PartSheet]) {
    x.open("part-list", &[]);
    // Families of two or more in a row share a bracket.
    let bracketed = |f: Family| matches!(f, Family::Woodwind | Family::Brass | Family::Strings | Family::Percussion);
    let mut group = 0u32;
    for (p, part) in parts.iter().enumerate() {
        let same_as = |q: Option<&PartSheet>| q.is_some_and(|q| q.family == part.family);
        let starts = bracketed(part.family) && !same_as(p.checked_sub(1).map(|q| &parts[q])) && same_as(parts.get(p + 1));
        if starts {
            group += 1;
            x.open("part-group", &[("type", "start"), ("number", &group.to_string())]);
            x.leaf("group-symbol", "bracket");
            x.leaf("group-barline", "yes");
            x.close("part-group");
        }

        let id = part_id(p);
        x.open("score-part", &[("id", &id)]);
        x.leaf("part-name", &part.name);
        x.leaf("part-abbreviation", &part.abbreviation);
        if part.percussion {
            for &key in &part.drums {
                x.open("score-instrument", &[("id", &instrument_id(p, Some(key)))]);
                x.leaf("instrument-name", gm::drum_name(key).unwrap_or("Percussion"));
                x.close("score-instrument");
            }
            for &key in &part.drums {
                x.open("midi-instrument", &[("id", &instrument_id(p, Some(key)))]);
                x.leaf("midi-channel", "10");
                x.leaf("midi-unpitched", &(u32::from(key) + 1).to_string());
                x.close("midi-instrument");
            }
        } else {
            x.open("score-instrument", &[("id", &instrument_id(p, None))]);
            x.leaf("instrument-name", &part.name);
            x.close("score-instrument");
            x.open("midi-instrument", &[("id", &instrument_id(p, None))]);
            x.leaf("midi-channel", &(u32::from(part.channel) + 1).to_string());
            x.leaf("midi-program", &(u32::from(part.program) + 1).to_string());
            x.close("midi-instrument");
        }
        x.close("score-part");

        let ends = bracketed(part.family) && same_as(p.checked_sub(1).map(|q| &parts[q])) && !same_as(parts.get(p + 1));
        if ends {
            x.empty("part-group", &[("type", "stop"), ("number", &group.to_string())]);
        }
    }
    x.close("part-list");
}

fn measure(x: &mut Xml, part: &PartSheet, p: usize, bar: &Bar, pb: &PartBar, first: bool) {
    let number = bar.number.to_string();
    let mut attrs = vec![("number", number.as_str())];
    if bar.implicit {
        attrs.push(("implicit", "yes"));
    }
    x.open("measure", &attrs);
    attributes(x, part, bar, pb, first);

    // Directions ride on the first voice of the top staff: the score-wide
    // ones (tempo, rehearsal marks) on the top part only.
    let mut directions: Vec<(u32, Direction)> = Vec::new();
    if p == 0 {
        directions.extend(bar.rehearsals.iter().map(|(o, t)| (*o, Direction::Rehearsal(t.clone()))));
        directions.extend(bar.tempos.iter().map(|(o, bpm)| (*o, Direction::Tempo(*bpm))));
    }
    directions.extend(pb.dynamics.iter().map(|(o, mark, v)| (*o, Direction::Dynamic(mark, *v))));
    directions.sort_by_key(|(o, _)| *o);

    for (v, voice) in pb.voices.iter().enumerate() {
        if v > 0 {
            x.open("backup", &[]);
            x.leaf("duration", &bar.len.to_string());
            x.close("backup");
        }
        for item in &voice.items {
            if v == 0 {
                for (o, d) in directions.iter().filter(|(o, _)| *o >= item.offset && *o < item.offset + item.len) {
                    direction(x, d, *o - item.offset, part.clefs.len() > 1);
                }
            }
            note(x, part, p, voice, item);
        }
    }
    x.close("measure");
}

fn attributes(x: &mut Xml, part: &PartSheet, bar: &Bar, pb: &PartBar, first: bool) {
    let key = (first || bar.show_key) && !(part.percussion && !first);
    let time = first || bar.show_meter;
    if !(first || key || time || pb.multi_rest.is_some()) {
        return;
    }
    x.open("attributes", &[]);
    if first {
        x.leaf("divisions", &super::DIV.to_string());
    }
    if key {
        x.open("key", &[]);
        if part.percussion {
            x.leaf("fifths", "0");
        } else {
            let (fifths, minor) = bar.key;
            x.leaf("fifths", &part.transposition.key(fifths).to_string());
            x.leaf("mode", if minor { "minor" } else { "major" });
        }
        x.close("key");
    }
    if time {
        x.open("time", &[]);
        x.leaf("beats", &bar.meter.num.to_string());
        x.leaf("beat-type", &bar.meter.den.to_string());
        x.close("time");
    }
    if first {
        if part.clefs.len() > 1 {
            x.leaf("staves", &part.clefs.len().to_string());
        }
        for (s, clef) in part.clefs.iter().enumerate() {
            let number = (s + 1).to_string();
            let attrs: Vec<(&str, &str)> = if part.clefs.len() > 1 { vec![("number", &number)] } else { Vec::new() };
            x.open("clef", &attrs);
            let (sign, line, octave) = clef.xml();
            x.leaf("sign", sign);
            if sign != "percussion" {
                x.leaf("line", &line.to_string());
            }
            if octave != 0 {
                x.leaf("clef-octave-change", &octave.to_string());
            }
            x.close("clef");
        }
        let t = part.transposition;
        if !t.is_none() {
            // MusicXML states it the other way: written to sounding.
            let octaves = t.octaves();
            let rest = (t.diatonic - 7 * octaves, t.chromatic - 12 * octaves);
            x.open("transpose", &[]);
            x.leaf("diatonic", &(-rest.0).to_string());
            x.leaf("chromatic", &(-rest.1).to_string());
            if octaves != 0 {
                x.leaf("octave-change", &(-octaves).to_string());
            }
            x.close("transpose");
        }
    }
    if let Some(n) = pb.multi_rest {
        x.open("measure-style", &[]);
        x.leaf("multiple-rest", &n.to_string());
        x.close("measure-style");
    }
    x.close("attributes");
}

enum Direction {
    Tempo(f64),
    Rehearsal(String),
    Dynamic(&'static str, u8),
}

fn direction(x: &mut Xml, d: &Direction, offset: u32, staves: bool) {
    let placement = if matches!(d, Direction::Dynamic(..)) { "below" } else { "above" };
    x.open("direction", &[("placement", placement)]);
    x.open("direction-type", &[]);
    match d {
        Direction::Tempo(bpm) => {
            x.open("metronome", &[("parentheses", "no")]);
            x.leaf("beat-unit", "quarter");
            x.leaf("per-minute", &format!("{}", bpm.round()));
            x.close("metronome");
        }
        Direction::Rehearsal(text) => x.leaf("rehearsal", text),
        Direction::Dynamic(mark, _) => {
            x.open("dynamics", &[]);
            x.empty(mark, &[]);
            x.close("dynamics");
        }
    }
    x.close("direction-type");
    if offset > 0 {
        x.leaf("offset", &offset.to_string());
    }
    if staves {
        x.leaf("staff", "1");
    }
    match d {
        Direction::Tempo(bpm) => x.empty("sound", &[("tempo", &trim(*bpm))]),
        Direction::Dynamic(_, v) => x.empty("sound", &[("dynamics", &trim(f64::from(*v) / 0.9))]),
        Direction::Rehearsal(_) => {}
    }
    x.close("direction");
}

fn trim(value: f64) -> String {
    let s = format!("{value:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn note(x: &mut Xml, part: &PartSheet, p: usize, voice: &VoiceBar, item: &Item) {
    let staves = part.clefs.len() > 1;
    if item.heads.is_empty() {
        x.open("note", &[]);
        if item.value.is_none() {
            x.empty("rest", &[("measure", "yes")]);
        } else {
            x.empty("rest", &[]);
        }
        x.leaf("duration", &item.len.to_string());
        x.leaf("voice", &voice.voice.to_string());
        value(x, item);
        if staves {
            x.leaf("staff", &voice.staff.to_string());
        }
        notations(x, item, false, false, true);
        x.close("note");
        return;
    }
    for (h, head) in item.heads.iter().enumerate() {
        // MusicXML's dynamics: percent of a forte at velocity 90.
        x.open("note", &[("dynamics", &trim(f64::from(head.velocity) / 0.9))]);
        if h > 0 {
            x.empty("chord", &[]);
        }
        match head.pitch {
            HeadPitch::Pitched(s) => {
                x.open("pitch", &[]);
                x.leaf("step", &s.letter().to_string());
                if s.alter != 0 {
                    x.leaf("alter", &s.alter.to_string());
                }
                x.leaf("octave", &s.octave.to_string());
                x.close("pitch");
            }
            HeadPitch::Unpitched { mark, .. } => {
                x.open("unpitched", &[]);
                x.leaf("display-step", &mark.step.to_string());
                x.leaf("display-octave", &mark.octave.to_string());
                x.close("unpitched");
            }
            HeadPitch::Sounding(_) => unreachable!("spelled before writing"),
        }
        x.leaf("duration", &item.len.to_string());
        if head.tie_stop {
            x.empty("tie", &[("type", "stop")]);
        }
        if head.tie_start {
            x.empty("tie", &[("type", "start")]);
        }
        if let HeadPitch::Unpitched { key, .. } = head.pitch {
            x.empty("instrument", &[("id", &instrument_id(p, Some(key)))]);
        }
        x.leaf("voice", &voice.voice.to_string());
        value(x, item);
        if let Some(acc) = head.accidental {
            x.leaf("accidental", acc);
        }
        if item.value.is_some_and(|v| v.triplet) {
            x.open("time-modification", &[]);
            x.leaf("actual-notes", "3");
            x.leaf("normal-notes", "2");
            x.close("time-modification");
        }
        if let Some(stem) = voice.stem {
            x.leaf("stem", stem);
        }
        if let HeadPitch::Unpitched { mark, .. } = head.pitch {
            if let Some(shape) = mark.notehead {
                x.leaf("notehead", shape);
            }
        }
        if staves {
            x.leaf("staff", &voice.staff.to_string());
        }
        if h == 0 {
            for (level, b) in item.beams.iter().enumerate() {
                x.open_inline("beam", &[("number", &(level + 1).to_string())]);
                x.text(b.name());
                x.close_inline("beam");
            }
        }
        notations(x, item, head.tie_stop, head.tie_start, h == 0);
        x.close("note");
    }
}

/// `<type>`, dots and (for rests and notes alike) nothing else.
fn value(x: &mut Xml, item: &Item) {
    if let Some(v) = item.value {
        x.leaf("type", v.kind.name());
        for _ in 0..v.dots {
            x.empty("dot", &[]);
        }
        if item.heads.is_empty() && v.triplet {
            x.open("time-modification", &[]);
            x.leaf("actual-notes", "3");
            x.leaf("normal-notes", "2");
            x.close("time-modification");
        }
    }
}

/// Ties, and the tuplet bracket's ends (on a chord's first note only).
fn notations(x: &mut Xml, item: &Item, tie_stop: bool, tie_start: bool, first: bool) {
    let tuplet_start = first && item.tuplet_start;
    let tuplet_stop = first && item.tuplet_stop;
    let staccato = first && item.staccato;
    if !(tie_stop || tie_start || tuplet_start || tuplet_stop || staccato) {
        return;
    }
    x.open("notations", &[]);
    if tie_stop {
        x.empty("tied", &[("type", "stop")]);
    }
    if tie_start {
        x.empty("tied", &[("type", "start")]);
    }
    if tuplet_start {
        x.empty("tuplet", &[("type", "start")]);
    }
    if tuplet_stop {
        x.empty("tuplet", &[("type", "stop")]);
    }
    if staccato {
        x.open("articulations", &[]);
        x.empty("staccato", &[]);
        x.close("articulations");
    }
    x.close("notations");
}

/* ---------- a small indenting writer ---------- */

#[derive(Default)]
struct Xml {
    out: String,
    depth: usize,
}

impl Xml {
    fn indent(&mut self) {
        for _ in 0..self.depth {
            self.out.push_str("  ");
        }
    }

    fn line(&mut self, raw: &str) {
        self.out.push_str(raw);
        self.out.push('\n');
    }

    fn tag(&mut self, name: &str, attrs: &[(&str, &str)], end: &str) {
        let _ = write!(self.out, "<{name}");
        for (k, v) in attrs {
            let _ = write!(self.out, " {k}=\"{}\"", escape(v));
        }
        self.out.push_str(end);
    }

    fn open(&mut self, name: &str, attrs: &[(&str, &str)]) {
        self.indent();
        self.tag(name, attrs, ">\n");
        self.depth += 1;
    }

    fn close(&mut self, name: &str) {
        self.depth -= 1;
        self.indent();
        let _ = writeln!(self.out, "</{name}>");
    }

    fn empty(&mut self, name: &str, attrs: &[(&str, &str)]) {
        self.indent();
        self.tag(name, attrs, "/>\n");
    }

    fn leaf(&mut self, name: &str, text: &str) {
        self.indent();
        let _ = writeln!(self.out, "<{name}>{}</{name}>", escape(text));
    }

    fn open_inline(&mut self, name: &str, attrs: &[(&str, &str)]) {
        self.indent();
        self.tag(name, attrs, ">");
    }

    fn text(&mut self, text: &str) {
        self.out.push_str(&escape(text));
    }

    fn close_inline(&mut self, name: &str) {
        let _ = writeln!(self.out, "</{name}>");
    }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}
