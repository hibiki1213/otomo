//! Note values: which written notes a length becomes, and how they beam.
//!
//! A length is not written as whatever note values add up to it. A copyist
//! shows the beat: a note that starts off the beat is tied at the next one,
//! a rest never hides where beat three is, and in 4/4 the middle of the bar
//! stays visible unless a note starts on the downbeat. The rules here are the
//! ordinary ones from engraving manuals, with the two syncopations everyone
//! writes as one note (eighth–quarter–eighth, quarter–half–quarter).

use super::DIV;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoteType {
    Whole,
    Half,
    Quarter,
    Eighth,
    N16th,
    N32nd,
    N64th,
}

impl NoteType {
    pub(crate) fn name(self) -> &'static str {
        match self {
            NoteType::Whole => "whole",
            NoteType::Half => "half",
            NoteType::Quarter => "quarter",
            NoteType::Eighth => "eighth",
            NoteType::N16th => "16th",
            NoteType::N32nd => "32nd",
            NoteType::N64th => "64th",
        }
    }

    /// Number of beams (or flags): 0 for a quarter and longer.
    pub(crate) fn beams(self) -> u8 {
        match self {
            NoteType::Whole | NoteType::Half | NoteType::Quarter => 0,
            NoteType::Eighth => 1,
            NoteType::N16th => 2,
            NoteType::N32nd => 3,
            NoteType::N64th => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Value {
    pub kind: NoteType,
    pub dots: u8,
    /// Three in the time of two.
    pub triplet: bool,
}

/// Single written notes, longest first, in divisions.
const STRAIGHT: [(u32, NoteType, u8); 12] = [
    (DIV * 4, NoteType::Whole, 0),
    (DIV * 3, NoteType::Half, 1),
    (DIV * 2, NoteType::Half, 0),
    (DIV * 3 / 2, NoteType::Quarter, 1),
    (DIV, NoteType::Quarter, 0),
    (DIV * 3 / 4, NoteType::Eighth, 1),
    (DIV / 2, NoteType::Eighth, 0),
    (DIV * 3 / 8, NoteType::N16th, 1),
    (DIV / 4, NoteType::N16th, 0),
    (DIV * 3 / 16, NoteType::N32nd, 1),
    (DIV / 8, NoteType::N32nd, 0),
    (DIV / 16, NoteType::N64th, 0),
];

/// Inside a triplet: what each length is written as (its 3:2 value).
const TRIPLET: [(u32, NoteType, u8); 5] = [
    (DIV * 2 / 3, NoteType::Quarter, 0),
    (DIV / 2, NoteType::Eighth, 1),
    (DIV / 3, NoteType::Eighth, 0),
    (DIV / 6, NoteType::N16th, 0),
    (DIV / 12, NoteType::N32nd, 0),
];

/// A triplet group always spans one quarter note.
pub(crate) const TRIPLET_SPAN: u32 = DIV;

fn straight(len: u32) -> Option<Value> {
    STRAIGHT
        .iter()
        .find(|(d, ..)| *d == len)
        .map(|&(_, kind, dots)| Value { kind, dots, triplet: false })
}

fn triplet(len: u32) -> Option<Value> {
    TRIPLET
        .iter()
        .find(|(d, ..)| *d == len)
        .map(|&(_, kind, dots)| Value { kind, dots, triplet: true })
}

/// The time signature as it governs note values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Meter {
    pub num: u8,
    pub den: u16,
}

impl Meter {
    /// Length of a full bar in divisions.
    pub(crate) fn len(self) -> u32 {
        u32::from(self.num) * DIV * 4 / u32::from(self.den)
    }

    /// 6/8, 9/8, 12/8 (and 3/8, counted in one): beats of three eighths.
    pub(crate) fn compound(self) -> bool {
        self.den >= 8 && self.num % 3 == 0
    }

    pub(crate) fn beat(self) -> u32 {
        let unit = DIV * 4 / u32::from(self.den);
        if self.compound() {
            unit * 3
        } else {
            unit
        }
    }

    /// 4/4 and 12/8 keep the middle of the bar visible.
    fn half_bar(self) -> Option<u32> {
        let halves = (!self.compound() && self.num == 4) || (self.compound() && self.num == 12);
        halves.then(|| self.len() / 2)
    }

    /// The first subdivision under the beat: eighths in 6/8, half beats otherwise.
    fn sub(self) -> u32 {
        if self.compound() {
            self.beat() / 3
        } else {
            self.beat() / 2
        }
    }

    /// The grid regions are chosen on: a quarter where triplets can live,
    /// otherwise the beat (or its half, for long beats).
    pub(crate) fn region(self) -> u32 {
        if !self.compound() && self.den <= 4 {
            TRIPLET_SPAN
        } else {
            self.beat().min(DIV * 3 / 2)
        }
    }

    /// Notes are beamed in these spans.
    pub(crate) fn beam_group(self) -> u32 {
        if self.compound() {
            self.beat()
        } else if self.den <= 8 {
            DIV
        } else {
            DIV / 2
        }
    }

    /// How strong a position in the bar is: 4 the downbeat, 3 the middle of
    /// a 4/4 bar, 2 a beat, 1 the first subdivision, then lower and lower.
    pub(crate) fn strength(self, pos: u32) -> i32 {
        for (unit, s) in self.levels() {
            if pos % unit == 0 {
                return s;
            }
        }
        -9
    }

    /// (spacing, strength) from the bar down to the finest note value.
    fn levels(self) -> Vec<(u32, i32)> {
        let mut out = vec![(self.len().max(1), 4)];
        if let Some(half) = self.half_bar() {
            out.push((half, 3));
        }
        out.push((self.beat(), 2));
        let mut unit = self.sub();
        let mut s = 1;
        while unit >= DIV / 16 && unit > 0 {
            out.push((unit, s));
            if unit % 2 != 0 {
                break;
            }
            unit /= 2;
            s -= 1;
        }
        out
    }

    /// The strongest bar position strictly between `o` and `e`, if any.
    fn strongest_inside(self, o: u32, e: u32) -> Option<(u32, i32)> {
        for (unit, s) in self.levels() {
            let b = (o / unit + 1) * unit;
            if b < e {
                return Some((b, s));
            }
        }
        None
    }
}

/// One written note value inside a bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Piece {
    pub offset: u32,
    pub len: u32,
    pub value: Value,
}

/// Where in a bar each quarter-note region uses triplets.
pub(crate) struct BarContext<'a> {
    pub meter: Meter,
    /// How far into a full bar this one starts: a pickup's notes fall at the
    /// end of the bar, metrically.
    pub shift: u32,
    pub triplets: &'a dyn Fn(u32) -> bool,
}

impl BarContext<'_> {
    /// Start (in this bar) of the triplet group `pos` is in, if it is in one.
    /// The closure is asked by metric position, so a pickup lines up.
    fn triplet_region(&self, pos: u32) -> Option<u32> {
        let r = (pos + self.shift) / TRIPLET_SPAN * TRIPLET_SPAN;
        let inside = self.meter.region() == TRIPLET_SPAN && r >= self.shift;
        (inside && (self.triplets)(r)).then(|| r - self.shift)
    }

    /// Write `[o, e)` (offsets in this bar) as note values, tied in order.
    pub(crate) fn spell(&self, o: u32, e: u32, rest: bool) -> Vec<Piece> {
        let mut out = Vec::new();
        self.spell_into(o, e, rest, &mut out);
        out
    }

    fn spell_into(&self, o: u32, e: u32, rest: bool, out: &mut Vec<Piece>) {
        if e <= o {
            return;
        }
        // A triplet group never shares a note with what is around it.
        if let Some(b) = self.triplet_edge_inside(o, e) {
            self.spell_into(o, b, rest, out);
            self.spell_into(b, e, rest, out);
            return;
        }
        let len = e - o;
        if let Some(r) = self.triplet_region(o) {
            let whole = o == r && len == TRIPLET_SPAN;
            if !whole {
                if let Some(value) = triplet(len) {
                    out.push(Piece { offset: o, len, value });
                    return;
                }
                // Split at the triplet's own beats: thirds, then sixths.
                let rel = o - r;
                for unit in [TRIPLET_SPAN / 3, TRIPLET_SPAN / 6, TRIPLET_SPAN / 12] {
                    let b = r + (rel / unit + 1) * unit;
                    if b < e {
                        self.spell_into(o, b, rest, out);
                        self.spell_into(b, e, rest, out);
                        return;
                    }
                }
                // Off every triplet position: nothing better than a 32nd.
                out.push(Piece { offset: o, len, value: Value { kind: NoteType::N32nd, dots: 0, triplet: true } });
                return;
            }
        }
        if let Some(value) = straight(len) {
            if self.allowed(o, e, rest) {
                out.push(Piece { offset: o, len, value });
                return;
            }
        }
        match self.meter.strongest_inside(o + self.shift, e + self.shift) {
            Some((b, _)) => {
                let b = b - self.shift;
                self.spell_into(o, b, rest, out);
                self.spell_into(b, e, rest, out);
            }
            // Finer than a 64th: write the nearest value rather than nothing.
            None => {
                let value = STRAIGHT
                    .iter()
                    .rev()
                    .find(|(d, ..)| *d >= len)
                    .map(|&(_, kind, dots)| Value { kind, dots, triplet: false })
                    .unwrap_or(Value { kind: NoteType::N64th, dots: 0, triplet: false });
                out.push(Piece { offset: o, len, value });
            }
        }
    }

    /// A triplet region boundary strictly inside `(o, e)`.
    fn triplet_edge_inside(&self, o: u32, e: u32) -> Option<u32> {
        if self.meter.region() != TRIPLET_SPAN {
            return None;
        }
        let (mo, me) = (o + self.shift, e + self.shift);
        let mut b = (mo / TRIPLET_SPAN + 1) * TRIPLET_SPAN;
        while b < me {
            // Only an edge if one side of it is a triplet.
            if (self.triplets)(b) || (self.triplets)(b - TRIPLET_SPAN) {
                return Some(b - self.shift);
            }
            b += TRIPLET_SPAN;
        }
        // A note starting inside a triplet region must end inside it too.
        None
    }

    /// Whether one note value may cover `[o, e)` here.
    fn allowed(&self, o: u32, e: u32, rest: bool) -> bool {
        let m = self.meter;
        let (mo, me) = (o + self.shift, e + self.shift);
        let start = m.strength(mo);
        let inside = m.strongest_inside(mo, me).map_or(i32::MIN, |(_, s)| s);
        // Within one beat anything goes: a sixteenth and a dotted eighth.
        if inside < 2 {
            return true;
        }
        if rest {
            return inside < start;
        }
        if start >= 2 {
            // From a beat, across beats; across the middle of 4/4 only from
            // the downbeat — or as quarter–half–quarter.
            return inside <= 2 || inside < start || (inside == start + 1 && e - o == 2 * m.beat());
        }
        // Eighth–quarter–eighth: from the half beat to the next half beat.
        start == 1 && !m.compound() && e - o == m.beat() && inside == 2
    }
}

/// How a note's beams connect, per level (index 0 is the eighth beam).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Beam {
    Begin,
    Continue,
    End,
    ForwardHook,
    BackwardHook,
}

impl Beam {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Beam::Begin => "begin",
            Beam::Continue => "continue",
            Beam::End => "end",
            Beam::ForwardHook => "forward hook",
            Beam::BackwardHook => "backward hook",
        }
    }
}

/// Beams for one run of beamable notes (each given as its beam count).
pub(crate) fn beam_run(levels: &[u8]) -> Vec<Vec<Beam>> {
    let n = levels.len();
    (0..n)
        .map(|i| {
            (1..=levels[i])
                .map(|k| {
                    let prev = i > 0 && levels[i - 1] >= k;
                    let next = i + 1 < n && levels[i + 1] >= k;
                    match (prev, next) {
                        (true, true) => Beam::Continue,
                        (true, false) => Beam::End,
                        (false, true) => Beam::Begin,
                        (false, false) if i == 0 => Beam::ForwardHook,
                        (false, false) => Beam::BackwardHook,
                    }
                })
                .collect()
        })
        .collect()
}
