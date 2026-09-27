//! Reading back what [`super::text`] writes.
//!
//! ```text
//! b5  C4@1:1 E4@2:1 [C4 E4 G4]@3:2v90
//! b6  -
//! ```
//!
//! Errors name the bar and the token, because this text is written by a model
//! that can only correct what it is told about.

use std::collections::BTreeMap;

use super::gm;

const NAMES: [(&str, i32); 7] =
    [("C", 0), ("D", 2), ("E", 4), ("F", 5), ("G", 7), ("A", 9), ("B", 11)];

/// One note as written: where in its bar, how long, how hard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Written {
    /// Beats from the start of the bar (0 = the downbeat).
    pub beat: f64,
    pub beats_long: f64,
    pub key: u8,
    pub velocity: u8,
}

/// The default when a token says nothing about velocity: an unremarkable
/// mezzo-forte, which is what a written note means.
const DEFAULT_VELOCITY: u8 = 80;

/// A note on a drum track may be named after the drum it plays.
fn parse_sound(text: &str, percussion: bool) -> Result<u8, String> {
    if percussion {
        if let Some(key) = gm::drum_key(text) {
            return Ok(key);
        }
    }
    parse_note_name(text).map_err(|e| {
        if percussion {
            format!("{e}（ドラムは Kick / Snare / HatClosed のような名前でも書けます）")
        } else {
            e
        }
    })
}

/// "C4" / "F#3" / "Bb2" → MIDI key.
pub fn parse_note_name(text: &str) -> Result<u8, String> {
    let bytes = text.as_bytes();
    let letter = bytes
        .first()
        .map(|b| b.to_ascii_uppercase() as char)
        .ok_or_else(|| "音名が空です".to_string())?;
    let mut semitone = NAMES
        .iter()
        .find(|(n, _)| n.as_bytes()[0] as char == letter)
        .map(|(_, s)| *s)
        .ok_or_else(|| format!("音名として読めません: {text}"))?;

    let mut rest = &text[1..];
    while let Some(c) = rest.chars().next() {
        match c {
            '#' | '♯' => semitone += 1,
            'b' | '♭' => semitone -= 1,
            _ => break,
        }
        rest = &rest[c.len_utf8()..];
    }
    let octave: i32 = rest
        .parse()
        .map_err(|_| format!("{text} にオクターブがありません（C4 のように書きます）"))?;
    let key = (octave + 1) * 12 + semitone;
    u8::try_from(key)
        .ok()
        .filter(|k| *k <= 127)
        .ok_or_else(|| format!("{text} は MIDI の音域外です（C-1〜G9）"))
}

/// Split a bar into tokens. Whitespace separates them, except inside the
/// brackets of a chord, which is where the renderer puts spaces.
fn tokenize(line: &str) -> Result<Vec<&str>, String> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let mut depth = 0usize;
        while i < bytes.len() && (depth > 0 || !bytes[i].is_ascii_whitespace()) {
            match bytes[i] {
                b'[' => depth += 1,
                b']' => {
                    depth = depth.checked_sub(1).ok_or_else(|| {
                        format!("{}: 和音の [ がありません", &line[start..=i])
                    })?
                }
                _ => {}
            }
            i += 1;
        }
        if depth > 0 {
            return Err(format!("{}: 和音の ] が閉じていません", &line[start..]));
        }
        out.push(&line[start..i]);
    }
    Ok(out)
}

/// Parse one bar's worth of tokens.
fn parse_bar(line: &str, percussion: bool) -> Result<Vec<Written>, String> {
    let mut out = Vec::new();
    for token in tokenize(line)? {
        if token == "-" || token == "." {
            continue; // an explicitly empty bar
        }
        out.extend(parse_token(token, percussion)?);
    }
    Ok(out)
}

fn parse_token(token: &str, percussion: bool) -> Result<Vec<Written>, String> {
    let (pitches, rest) = match token.strip_prefix('[') {
        Some(after) => {
            let (inside, rest) = after
                .split_once(']')
                .ok_or_else(|| format!("{token}: 和音の ] が閉じていません"))?;
            (
                inside
                    .split(|c: char| c.is_whitespace() || c == ',')
                    .filter(|p| !p.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
                rest,
            )
        }
        None => {
            let at = token
                .find('@')
                .ok_or_else(|| format!("{token}: `音名@拍:長さ` の形式で書きます（例 C4@1:1）"))?;
            (vec![token[..at].to_owned()], &token[at..])
        }
    };

    let rest = rest
        .strip_prefix('@')
        .ok_or_else(|| format!("{token}: 位置の前に @ が要ります（例 [C4 E4]@1:2）"))?;
    // The velocity suffix is optional: `...:2v90`.
    let (timing, velocity) = match rest.rsplit_once('v') {
        Some((timing, vel)) => (
            timing,
            vel.parse::<u8>()
                .map_err(|_| format!("{token}: 強さは 1〜127 の数字です"))?
                .clamp(1, 127),
        ),
        None => (rest, DEFAULT_VELOCITY),
    };
    let (beat, length) = timing
        .split_once(':')
        .ok_or_else(|| format!("{token}: 長さが要ります（例 C4@1:0.5）"))?;
    let beat: f64 = beat.parse().map_err(|_| format!("{token}: 拍が数字ではありません"))?;
    let beats_long: f64 =
        length.parse().map_err(|_| format!("{token}: 長さが数字ではありません"))?;
    if beat < 1.0 {
        return Err(format!("{token}: 拍は小節の頭を 1 として数えます"));
    }
    if beats_long <= 0.0 {
        return Err(format!("{token}: 長さは 0 より大きい必要があります"));
    }

    pitches
        .iter()
        .map(|p| {
            Ok(Written {
                beat: beat - 1.0,
                beats_long,
                key: parse_sound(p, percussion)?,
                velocity,
            })
        })
        .collect()
}

/// Parse a passage: one line per bar, each optionally labelled `b12`.
///
/// Unlabelled lines continue from `first_bar`, so a model can write either
/// form. The result is keyed by bar number.
pub fn parse_passage(
    text: &str,
    first_bar: u32,
    percussion: bool,
) -> Result<BTreeMap<u32, Vec<Written>>, String> {
    let mut out: BTreeMap<u32, Vec<Written>> = BTreeMap::new();
    let mut next = first_bar;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // `b12  C4@1:1` — the label is optional but wins when present.
        let (bar, body) = match line.split_once(char::is_whitespace) {
            Some((head, body)) if is_bar_label(head) => (bar_number(head)?, body),
            _ if is_bar_label(line) => (bar_number(line)?, ""),
            _ => (next, line),
        };
        let notes = parse_bar(body, percussion).map_err(|e| format!("{bar}小節目: {e}"))?;
        out.entry(bar).or_default().extend(notes);
        next = bar + 1;
    }
    Ok(out)
}

fn is_bar_label(word: &str) -> bool {
    word.strip_prefix('b').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

fn bar_number(word: &str) -> Result<u32, String> {
    word[1..].parse().map_err(|_| format!("小節番号として読めません: {word}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_names_round_trip() {
        for key in 0u8..=127 {
            let name = crate::note_name(key);
            assert_eq!(parse_note_name(&name).unwrap(), key, "{name}");
        }
        assert_eq!(parse_note_name("Bb3").unwrap(), parse_note_name("A#3").unwrap());
        assert!(parse_note_name("H4").is_err());
        assert!(parse_note_name("C").is_err());
        assert!(parse_note_name("C99").is_err());
    }

    #[test]
    fn reads_what_the_renderer_writes() {
        let passage = parse_passage("b5  C4@1:1 E4@2:1 [C4 E4 G4]@3:2v90\nb6  -\n", 1, false).unwrap();
        let five = &passage[&5];
        assert_eq!(five.len(), 5);
        assert_eq!(five[0], Written { beat: 0.0, beats_long: 1.0, key: 60, velocity: 80 });
        assert_eq!(five[2].beat, 2.0);
        assert_eq!(five[2].velocity, 90);
        assert!(passage[&6].is_empty(), "a bar of rest is still a bar");
    }

    #[test]
    fn unlabelled_lines_follow_on() {
        let passage = parse_passage("C4@1:1\nD4@1:1\n\nE4@1:1\n", 10, false).unwrap();
        assert_eq!(passage.keys().copied().collect::<Vec<_>>(), vec![10, 11, 12]);
        assert_eq!(passage[&12][0].key, 64);
    }

    #[test]
    fn says_what_is_wrong_and_where() {
        let err = parse_passage("b3  C4@1", 1, false).unwrap_err();
        assert!(err.contains("3小節目") && err.contains("長さ"), "{err}");
        let err = parse_passage("b3  [C4 E4@1:1", 1, false).unwrap_err();
        assert!(err.contains("]"), "{err}");
        let err = parse_passage("b3  C4@0:1", 1, false).unwrap_err();
        assert!(err.contains("小節の頭"), "{err}");
        let err = parse_passage("b3  C4@1:0", 1, false).unwrap_err();
        assert!(err.contains("0 より大きい"), "{err}");
    }
}
