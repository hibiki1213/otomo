//! The editing half of the tool set.
//!
//! Every one of these follows the same shape: read the current version (the
//! staged one if an edit is already waiting), change one thing, stage the
//! result. Nothing here writes to the library — a person applies the change
//! in the app, having heard it.

use otomo_score::{program_number, KeyChange, Score, Voice};
use serde_json::{json, Value};

use crate::control;
use crate::library::{Library, Song};
use crate::pending;
use crate::rpc::{need_i64, need_str, opt_i64, opt_str, Tool};

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "create_song",
            description: "空の曲をライブラリに作る。作ったあと add_track と write_bars で中身を書く。\
                          既にある曲（国歌・クラシック・民謡など）なら、先にウェブで MIDI や楽譜を探して download で取り込む。",
            schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "tempo": { "type": "number", "description": "BPM（既定 120）" },
                    "time_signature": { "type": "string", "description": "\"4/4\"（既定）、\"3/4\" など" },
                    "key": { "type": "string", "description": "\"D major\"、\"B minor\" など" }
                },
                "required": ["title"]
            }),
        },
        Tool {
            name: "write_bars",
            description: "1トラックの指定した小節範囲を、書いたとおりに置き換える。\
                          記法は read_bars と同じ `音名@拍:長さ`（強さは `v90` を付ける）。\
                          範囲内で書かなかった小節は空になる。同じ呼び出しを繰り返しても二重にならない。",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "track": { "type": "integer", "description": "get_song のトラック番号" },
                    "from_bar": { "type": "integer" },
                    "to_bar": { "type": "integer" },
                    "notes": {
                        "type": "string",
                        "description": "1行1小節。`b12  C4@1:1 [E4 G4]@3:2` のように小節番号を付ける。\
                                        空文字列にすると範囲を消す。"
                    }
                },
                "required": ["song_id", "track", "from_bar", "to_bar", "notes"]
            }),
        },
        Tool {
            name: "add_track",
            description: "楽器トラックを足す。percussion を true にするとドラム（MIDI ch.10）になる。",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "instrument": {
                        "type": "string",
                        "description": "General MIDI の楽器名（\"Flute\"、\"French Horn\"、\"String Ensemble 1\"）か番号 0〜127"
                    },
                    "name": { "type": "string", "description": "譜面上のパート名（省略すると楽器名）" },
                    "percussion": { "type": "boolean" }
                },
                "required": ["song_id", "instrument"]
            }),
        },
        Tool {
            name: "set_track",
            description: "トラックの楽器や名前を変える。",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "track": { "type": "integer" },
                    "instrument": { "type": "string", "description": "General MIDI の楽器名か番号" },
                    "name": { "type": "string" }
                },
                "required": ["song_id", "track"]
            }),
        },
        Tool {
            name: "add_tracks",
            description: "楽器をまとめて足す。編成を一度に組むとき用。\
                          \"Drums\" を含めるとドラムトラックになる。",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "instruments": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "例: [\"Flute\", \"Clarinet\", \"French Horn\", \"String Ensemble 1\", \"Drums\"]"
                    }
                },
                "required": ["song_id", "instruments"]
            }),
        },
        Tool {
            name: "write_score",
            description: "複数トラックをまとめて書く。read_bars の出力と同じ形式で、\
                          `[0] Flute` の見出しで次のトラックに切り替わる。\
                          見出しに出てこないトラックは触らない。編成が大きいときはこちらを使う。",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "from_bar": { "type": "integer" },
                    "to_bar": { "type": "integer" },
                    "notes": {
                        "type": "string",
                        "description": "例:\n[0] Flute\nb1  C6@1:2 D6@3:2\nb2  E6@1:4\n\n[2] Cello\nb1  C3@1:4\nb2  G2@1:4"
                    }
                },
                "required": ["song_id", "from_bar", "to_bar", "notes"]
            }),
        },
        Tool {
            name: "copy_bars",
            description: "あるトラックの小節範囲を別のトラックへ写す。オーケストレーションの主作業。\
                          transpose で移調、voice で和音の上声部（melody）や下声部（bass）だけを取り出せる。\
                          merge を true にすると置き換えずに重ねる（オクターブ重複など）。",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "from_track": { "type": "integer" },
                    "to_track": { "type": "integer" },
                    "from_bar": { "type": "integer" },
                    "to_bar": { "type": "integer" },
                    "at_bar": { "type": "integer", "description": "写し先の先頭小節（省略すると同じ小節）" },
                    "transpose": { "type": "integer", "description": "半音。12 で1オクターブ上" },
                    "voice": {
                        "type": "string",
                        "description": "\"all\"（既定）/ \"top\"（和音の最高音＝旋律）/ \"bottom\"（最低音＝バス）"
                    },
                    "merge": { "type": "boolean", "description": "写し先を消さずに重ねる（既定 false）" }
                },
                "required": ["song_id", "from_track", "to_track", "from_bar", "to_bar"]
            }),
        },
        Tool {
            name: "remove_track",
            description: "トラックを丸ごと消す。以降トラック番号が繰り上がる点に注意。",
            schema: json!({
                "type": "object",
                "properties": { "song_id": { "type": "integer" }, "track": { "type": "integer" } },
                "required": ["song_id", "track"]
            }),
        },
        Tool {
            name: "set_tempo",
            description: "テンポを変える。from_bar を指定するとその小節から。\
                          to_bar と to_bpm を両方指定すると、その区間で徐々に変わる（リタルダンド等）。",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "bpm": { "type": "number" },
                    "from_bar": { "type": "integer" },
                    "to_bar": { "type": "integer", "description": "to_bpm と一緒に指定すると、その範囲で徐々に変化する（rit. / accel.）" },
                    "to_bpm": { "type": "number" }
                },
                "required": ["song_id", "bpm"]
            }),
        },
        Tool {
            name: "set_time_signature",
            description: "拍子を変える。from_bar 以降の小節番号が変わる。",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "signature": { "type": "string", "description": "\"3/4\"、\"6/8\" など" },
                    "from_bar": { "type": "integer" }
                },
                "required": ["song_id", "signature"]
            }),
        },
        Tool {
            name: "set_key",
            description: "調号を設定する。",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "key": { "type": "string", "description": "\"D major\"、\"B minor\" など" }
                },
                "required": ["song_id", "key"]
            }),
        },
        Tool {
            name: "insert_bars",
            description: "指定した小節の前に空の小節を差し込む。以降の音は後ろにずれる。",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "at_bar": { "type": "integer" },
                    "count": { "type": "integer" }
                },
                "required": ["song_id", "at_bar", "count"]
            }),
        },
        Tool {
            name: "delete_bars",
            description: "小節を全トラックから取り除く。以降の音は前に詰まる。",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "from_bar": { "type": "integer" },
                    "to_bar": { "type": "integer" }
                },
                "required": ["song_id", "from_bar", "to_bar"]
            }),
        },
        Tool {
            name: "set_dynamics",
            description: "小節範囲の強弱を決める。to_level を付けるとクレッシェンド／ディミヌエンドになり、\
                          伸ばした音も膨らむようエクスプレッション（CC11）の曲線も書く。",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "track": { "type": "integer" },
                    "from_bar": { "type": "integer" },
                    "to_bar": { "type": "integer" },
                    "level": { "type": "string", "description": "pp / p / mp / mf / f / ff か 1〜127" },
                    "to_level": { "type": "string", "description": "省略すると一定。指定するとその強さへ向かって変化する" }
                },
                "required": ["song_id", "track", "from_bar", "to_bar", "level"]
            }),
        },
        Tool {
            name: "set_pickup",
            description: "1小節目を弱起（アウフタクト）にする。beats に弱起の拍数。0 にすると解除。\
                          以降の小節番号が1つずれる点に注意。",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "beats": { "type": "number", "description": "弱起の長さ（拍）" }
                },
                "required": ["song_id", "beats"]
            }),
        },
        Tool {
            name: "discard_changes",
            description: "未適用の編集を捨てて、ライブラリにある状態へ戻す。",
            schema: json!({
                "type": "object",
                "properties": { "song_id": { "type": "integer" } },
                "required": ["song_id"]
            }),
        },
    ]
}

pub fn is_edit(name: &str) -> bool {
    tools().iter().any(|t| t.name == name)
}

/// Run an editing tool. `open` hands back the song and the version to edit.
pub fn call(
    name: &str,
    args: &Value,
    library: &Library,
    data_dir: &std::path::Path,
) -> Result<String, String> {
    if name == "create_song" {
        return create_song(args, library, data_dir);
    }
    let song = library.get(need_i64(args, "song_id")?)?;
    if name == "discard_changes" {
        return Ok(if pending::discard(data_dir, song.id) {
            notify(data_dir, song.id);
            format!("「{}」の未適用の編集を捨てました。", song.title)
        } else {
            format!("「{}」に未適用の編集はありません。", song.title)
        });
    }

    let mut score = pending::current(library, data_dir, &song)?;
    let summary = apply(name, args, &mut score)?;
    let staged = pending::stage(data_dir, &song, &score, &summary)?;
    notify(data_dir, song.id);
    Ok(staged)
}

/// Tell the app an edit is waiting, so its banner appears at once. Best
/// effort: with the app closed the files are still there to be found later.
fn notify(data_dir: &std::path::Path, song_id: i64) {
    let _ = control::send(data_dir, json!({ "cmd": "staged", "songId": song_id }));
}

fn apply(name: &str, args: &Value, score: &mut Score) -> Result<String, String> {
    match name {
        "write_bars" => score.write_bars(
            track_of(args)?,
            need_bar(args, "from_bar")?,
            need_bar(args, "to_bar")?,
            need_str(args, "notes")?,
        ),
        "add_track" => {
            let percussion = args.get("percussion").and_then(Value::as_bool).unwrap_or(false);
            let instrument = instrument_of(args, "instrument")?.unwrap_or(0);
            // An unnamed part is called after what it plays, which reads
            // better in the app than "Track 4".
            let name = opt_str(args, "name")
                .unwrap_or_else(|| otomo_score::program_name(instrument).to_string());
            Ok(score.add_track(instrument, &name, percussion))
        }
        "add_tracks" => {
            let list = args
                .get("instruments")
                .and_then(Value::as_array)
                .ok_or("instruments は必須です（楽器名の配列）")?;
            if list.is_empty() {
                return Err("楽器を1つ以上指定してください。".into());
            }
            let mut done = Vec::new();
            for item in list {
                let text = item.as_str().ok_or("instruments は文字列の配列です")?;
                let percussion = text.trim().eq_ignore_ascii_case("drums")
                    || text.trim().eq_ignore_ascii_case("percussion");
                let program = if percussion { 0 } else { program_number(text)? };
                let name = if percussion {
                    "Drums".to_string()
                } else {
                    otomo_score::program_name(program).to_string()
                };
                done.push(score.add_track(program, &name, percussion));
            }
            Ok(done.join("\n"))
        }
        "set_track" => score.set_track(
            track_of(args)?,
            instrument_of(args, "instrument")?,
            opt_str(args, "name").as_deref(),
        ),
        "copy_bars" => score.copy_bars(
            need_usize(args, "from_track")?,
            need_usize(args, "to_track")?,
            need_bar(args, "from_bar")?,
            need_bar(args, "to_bar")?,
            match opt_i64(args, "at_bar") {
                Some(_) => need_bar(args, "at_bar")?,
                None => need_bar(args, "from_bar")?,
            },
            opt_i64(args, "transpose").unwrap_or(0).clamp(-48, 48) as i32,
            match opt_str(args, "voice") {
                Some(text) => Voice::parse(&text)?,
                None => Voice::All,
            },
            args.get("merge").and_then(Value::as_bool).unwrap_or(false),
        ),
        "write_score" => score.write_score(
            need_bar(args, "from_bar")?,
            need_bar(args, "to_bar")?,
            need_str(args, "notes")?,
        ),
        "remove_track" => score.remove_track(track_of(args)?),
        "set_tempo" => {
            let bpm = args.get("bpm").and_then(Value::as_f64).ok_or("bpm は必須です（数値）")?;
            let from = opt_i64(args, "from_bar").unwrap_or(1).max(1) as u32;
            // A second tempo and a range make it a curve rather than a step.
            match (args.get("to_bpm").and_then(Value::as_f64), opt_i64(args, "to_bar")) {
                (Some(to_bpm), Some(_)) => score.tempo_curve(from, need_bar(args, "to_bar")?, bpm, to_bpm),
                _ => score.set_tempo(bpm, from),
            }
        }
        "set_dynamics" => score.set_dynamics(
            track_of(args)?,
            need_bar(args, "from_bar")?,
            need_bar(args, "to_bar")?,
            need_str(args, "level")?,
            opt_str(args, "to_level").as_deref(),
        ),
        "set_pickup" => score.set_pickup(
            args.get("beats").and_then(Value::as_f64).ok_or("beats は必須です（拍数）")?,
        ),
        "set_time_signature" => {
            let (num, den) = parse_signature(need_str(args, "signature")?)?;
            score.set_time_signature(num, den, opt_i64(args, "from_bar").unwrap_or(1).max(1) as u32)
        }
        "set_key" => {
            let (sharps, minor) = KeyChange::parse(need_str(args, "key")?)?;
            score.set_key(sharps, minor)
        }
        "insert_bars" => score.insert_bars(
            need_bar(args, "at_bar")?,
            need_i64(args, "count")?.clamp(0, 512) as u32,
        ),
        "delete_bars" => score.delete_bars(need_bar(args, "from_bar")?, need_bar(args, "to_bar")?),
        other => Err(format!("知らないツールです: {other}")),
    }
}

fn create_song(args: &Value, library: &Library, data_dir: &std::path::Path) -> Result<String, String> {
    let (num, den) = match opt_str(args, "time_signature") {
        Some(text) => parse_signature(&text)?,
        None => (4, 4),
    };
    let bpm = args.get("tempo").and_then(Value::as_f64).unwrap_or(120.0);
    if !(10.0..=400.0).contains(&bpm) {
        return Err("テンポは 10〜400 BPM の範囲で指定してください。".into());
    }
    let mut score = Score::blank(bpm, num, den);
    if let Some(key) = opt_str(args, "key") {
        let (sharps, minor) = KeyChange::parse(&key)?;
        score.set_key(sharps, minor)?;
    }
    let title = need_str(args, "title")?;
    let song = library.create(title, &score.to_smf()?)?;
    let _ = data_dir;
    Ok(format!(
        "「{title}」を作りました（song_id {}、{bpm:.0} BPM、{num}/{den}）。\
         add_track で楽器を足してから write_bars で書いてください。",
        song.id
    ))
}

fn track_of(args: &Value) -> Result<usize, String> {
    need_usize(args, "track")
}

fn need_usize(args: &Value, key: &str) -> Result<usize, String> {
    usize::try_from(need_i64(args, key)?).map_err(|_| format!("{key} は 0 以上です"))
}

/// An instrument given by name or by number; absent means "leave it alone".
fn instrument_of(args: &Value, key: &str) -> Result<Option<u8>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => Ok(Some(n.as_i64().unwrap_or(0).clamp(0, 127) as u8)),
        Some(Value::String(text)) => program_number(text).map(Some),
        Some(_) => Err(format!("{key} は楽器名か 0〜127 の番号です")),
    }
}

fn need_bar(args: &Value, key: &str) -> Result<u32, String> {
    let bar = need_i64(args, key)?;
    u32::try_from(bar)
        .ok()
        .filter(|b| *b >= 1)
        .ok_or_else(|| format!("{key} は 1 以上の小節番号です"))
}

/// "6/8" → (6, 8).
fn parse_signature(text: &str) -> Result<(u8, u16), String> {
    let (num, den) = text
        .trim()
        .split_once('/')
        .ok_or_else(|| format!("{text}: 拍子は \"4/4\" のように書きます"))?;
    let num: u8 = num.trim().parse().map_err(|_| format!("{text}: 分子が数字ではありません"))?;
    let den: u16 = den.trim().parse().map_err(|_| format!("{text}: 分母が数字ではありません"))?;
    Ok((num, den))
}

/// What is staged, for `get_song` and the `pending_changes` tool.
pub fn describe_staged(data_dir: &std::path::Path, song: &Song) -> Option<String> {
    let staged = pending::staged(data_dir, song.id)?;
    let mut out = format!("\n未適用の編集（Otomo で「適用」を押すと確定）:\n");
    for change in &staged.changes {
        out.push_str(&format!("  ・{}\n", change.lines().next().unwrap_or(change)));
    }
    Some(out)
}
