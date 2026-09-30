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
                description: "Otomo のライブラリにある曲の一覧。song_id はほかのツールで使う。",
                schema: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "曲名の部分一致で絞り込む" }
                    }
                }),
            },
            Tool {
                name: "download",
                description: "URL のファイルを取ってくる。国歌・クラシック・民謡・スタンダードなど既にある曲は、\
                              記憶で書かずに、ウェブで楽譜や MIDI を探してここで取り込む。\n\
                              ・MusicXML（.musicxml / .xml / .mxl）→ 楽譜の音符をそのまま曲にして取り込む。いちばん正確。\n\
                              ・MIDI（.mid）→ ライブラリに新しい曲として取り込む。\n\
                              どちらも取り込んだら get_song と同じ概要を返すので、read_bars で確かめてから編成やアレンジを直す。\n\
                              ・PDF・画像（PNG / JPEG）の楽譜 → ページを画像で返す。読み取って create_song と write_score で書き起こす。\
                              MusicXML や MIDI が見つからないときの手段。pages で読むページを選ぶ（一度に3ページまで）。\n\
                              ・テキスト（ABC 記法、歌詞など）→ 中身を加工せずに返す。長いものは offset で続きを読む。\n\
                              HTML のページは WebFetch で読む。SVG の楽譜は PNG に変換された版の URL を使う。",
                schema: json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "ファイルそのものの URL（http / https）" },
                        "title": { "type": "string", "description": "取り込むときの曲名（省略すると楽譜の題名かファイル名）" },
                        "pages": { "type": "string", "description": "PDF で読むページ。\"1-2\"（既定）、\"3\" など" },
                        "offset": { "type": "integer", "description": "テキストの続きを読むときの開始文字位置" }
                    },
                    "required": ["url"]
                }),
            },
            Tool {
                name: "get_song",
                description: "曲の全体像：トラック（楽器・音域・音数）、調、拍子、テンポ、小節数。\
                              read_bars で中身を読む前に、まずこれを見る。",
                schema: json!({
                    "type": "object",
                    "properties": { "song_id": { "type": "integer" } },
                    "required": ["song_id"]
                }),
            },
            Tool {
                name: "read_bars",
                description: format!(
                    "指定した小節範囲の音符を読む。1小節1行、`音名@拍:長さ` の形式で、\
                     拍は小節内の1始まり、長さは拍数。同時に始まり同じ長さの音は `[C4 E4 G4]` とまとめる。\
                     人間の演奏を録った MIDI は、その範囲に合う単位に丸めて読む（quantize で変えられる）。\
                     一度に読めるのは{MAX_BARS}小節まで。"
                )
                .leak(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "song_id": { "type": "integer" },
                        "from_bar": { "type": "integer", "description": "1始まり" },
                        "to_bar": { "type": "integer" },
                        "tracks": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "description": "get_song のトラック番号。省略すると全トラック"
                        },
                        "velocity": { "type": "boolean", "description": "強さも出す（既定: false）" },
                        "quantize": {
                            "type": "string",
                            "description": "位置と長さを丸める単位。省略するとその範囲に合う単位を自動で選ぶ。\"1/16\"、\"1/12\"（三連符）、\"off\" で丸めない"
                        }
                    },
                    "required": ["song_id", "from_bar", "to_bar"]
                }),
            },
            Tool {
                name: "harmony",
                description: "小節範囲の和声進行をコードネームで読む。\
                              パートが多い曲で「ここは何をしているか」を掴むには、read_bars より桁違いに軽い。\
                              拍ごとに判定し、続いている和音はまとめる。",
                schema: json!({
                    "type": "object",
                    "properties": {
                        "song_id": { "type": "integer" },
                        "from_bar": { "type": "integer" },
                        "to_bar": { "type": "integer" },
                        "tracks": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "description": "省略すると全トラック（打楽器は除外される）"
                        }
                    },
                    "required": ["song_id", "from_bar", "to_bar"]
                }),
            },
            Tool {
                name: "play",
                description: "起動中の Otomo でこの曲を鳴らす。音を確かめてもらうために使う。\
                              to_bar で範囲を区切り、loop で繰り返せる（直して聴き直す作業向け）。\
                              tracks を指定するとそのパートだけが鳴る（ソロ）。",
                schema: json!({
                    "type": "object",
                    "properties": {
                        "song_id": { "type": "integer" },
                        "from_bar": { "type": "integer" },
                        "to_bar": { "type": "integer", "description": "ここまで鳴らして止まる" },
                        "loop": { "type": "boolean", "description": "範囲を繰り返す" },
                        "tracks": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "description": "このパートだけ鳴らす。省略すると全部"
                        }
                    },
                    "required": ["song_id"]
                }),
            },
            Tool {
                name: "list_playback_instruments",
                description: "Otomo で使える音源（内蔵シンセ、SFZ ライブラリ、AudioUnit プラグイン）の一覧。\
                              内蔵シンセには EDM 向けの音色（スーパーソウ、プラック、808、ワブルベース、EDM ドラムなど）がある。\
                              General MIDI の音色より良い音で鳴らしたいときに使う。起動中の Otomo が要る。",
                schema: json!({ "type": "object", "properties": {} }),
            },
            Tool {
                name: "set_playback_instrument",
                description: "いま開いている曲のトラックを、指定した音源で鳴らす。\
                              これは再生の設定で、曲そのものは変わらない（保存もされない）。",
                schema: json!({
                    "type": "object",
                    "properties": {
                        "track": { "type": "integer" },
                        "instrument": { "type": "string", "description": "list_playback_instruments の id" }
                    },
                    "required": ["track", "instrument"]
                }),
            },
            Tool {
                name: "stop",
                description: "再生を止める。",
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
                "「{}」{}を{bar}小節目（{}）から{}{}再生しています。",
                song.title,
                if staged { "（未適用の編集を含む版）" } else { "" },
                clock(seconds),
                match (opt_i64(args, "to_bar"), looping) {
                    (Some(to), true) => format!("{to}小節目まで繰り返して"),
                    (Some(to), false) => format!("{to}小節目まで"),
                    (None, _) => String::new(),
                },
                match &solo {
                    Some(list) if !list.is_empty() => format!(
                        "（{}のみ）",
                        list.iter()
                            .filter_map(|i| score.tracks.get(*i))
                            .map(track_label)
                            .collect::<Vec<_>>()
                            .join("・")
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
                usize::try_from(need_i64(args, "track")?).map_err(|_| "track は 0 以上です")?,
                need_str(args, "instrument")?,
            )?;
            Ok(reply
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("音源を切り替えました。")
                .to_string())
        }
        "stop" => {
            control::stop(&dir)?;
            Ok("再生を止めました。".into())
        }
        other if edit_tools::is_edit(other) => {
            let library = Library::open(&dir)?;
            edit_tools::call(other, args, &library, &dir)
        }
        other => Err(format!("知らないツールです: {other}")),
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
            let score = Score::parse(&bytes).map_err(|e| format!("MIDI として読めませんでした: {e}"))?;
            let title = named.or_else(|| fetch::title_from(url)).unwrap_or_else(|| "ダウンロードした曲".into());
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
                .unwrap_or_else(|| "ダウンロードした曲".into());
            import(dir, &title, &smf, &score, url, &read.remarks)
        }
        Fetched::Pdf(bytes) => {
            let (first, last) = page_range(args)?;
            let pages = picture::pdf_pages(&bytes, first, last)?;
            let shown = pages.images.iter().map(|(n, _)| *n);
            let (from, to) = (shown.clone().min().unwrap_or(first), shown.max().unwrap_or(first));
            let mut text = format!(
                "{url} — PDF の楽譜、全{}ページのうち {from}〜{to} ページ目です。\n\n{READ_BY_EYE}",
                pages.count
            );
            if to < pages.count {
                let next = (to + 1, (to + MAX_PAGES).min(pages.count));
                text.push_str(&format!("\n続きは pages: \"{}-{}\" で読めます。", next.0, next.1));
            }
            Ok(Reply { text, images: pages.images.into_iter().map(|(_, png)| ("image/png", png)).collect() })
        }
        Fetched::Scan(bytes) => Ok(Reply {
            text: format!("{url} — 楽譜の画像です。\n\n{READ_BY_EYE}"),
            images: vec![("image/png", picture::scan(&bytes)?)],
        }),
        Fetched::Text(text) => Ok(text_page(url, &text, opt_i64(args, "offset").unwrap_or(0)).into()),
        Fetched::Html => Err("HTML のページです。内容は WebFetch で読み、そこから楽譜ファイルそのもの\
                              （.musicxml / .mxl / .mid / .pdf など）へのリンクを探して download に渡してください。"
            .into()),
        Fetched::Svg => Err("SVG の楽譜は読めません。PNG に変換された版の URL を使ってください\
                             （Wikimedia Commons なら upload.wikimedia.org/…/thumb/…/1600px-….svg.png）。"
            .into()),
        Fetched::Other(kind) => Err(format!(
            "{kind} のファイルは読めません。MusicXML・MIDI・PDF・PNG / JPEG の楽譜か、ABC 記法などのテキストを指定してください。"
        )),
    }
}

/// What to do with a score that arrives as pictures.
const READ_BY_EYE: &str = "読み取って書き起こしてください。\n\
    ・先に調号・拍子・テンポ・パート構成・弱起の有無を確かめる。\n\
    ・create_song → add_tracks → write_score で、数小節ずつ書く。移調楽器の譜は実音に直す。\n\
    ・読み取れない、自信のない箇所は推測で埋めず、返事でそう伝える。";

fn import(
    dir: &std::path::Path,
    title: &str,
    smf: &[u8],
    score: &Score,
    url: &str,
    remarks: &[String],
) -> Result<Reply, String> {
    let song = Library::open(dir)?.create(title, smf)?;
    let mut text = format!("「{}」として取り込みました（song_id {}、出典 {url}）。\n", song.title, song.id);
    for remark in remarks {
        text.push_str(&format!("※ {remark}\n"));
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
    let bad = || format!("{text}: pages は \"1-2\" や \"3\" のように書きます");
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
        format!("{url}（{total}文字）\n\n")
    } else {
        format!("{url}（全{total}文字のうち {from}〜{to}）\n\n")
    };
    out.push_str(&body);
    if to < total {
        out.push_str(&format!("\n\n…続きがあります。offset: {to} で続きを読めます。"));
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
            Some(q) => format!("「{q}」に一致する曲はありません。"),
            None => "ライブラリはまだ空です。Otomo で MIDI を開くと追加されます。".into(),
        });
    }
    let mut out = String::from("song_id  長さ   トラック  曲名\n");
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
    let mut out = format!("{}（{}）\n", song.title, song.file_name);
    out.push_str(&format!(
        "長さ {} / {}小節 / テンポ {} / {}\n",
        clock(score.duration()),
        bars.count,
        if lo == hi { format!("{lo} BPM") } else { format!("{lo}–{hi} BPM") },
        signature_summary(score, &bars),
    ));
    if let Some(k) = score.key_signatures.first() {
        out.push_str(&format!("調 {}\n", k.name()));
    }
    if let Some(beats) = score.looks_like_a_pickup() {
        out.push_str(&format!(
            "※ 最初の音が1小節目の途中（残り{beats}拍）から始まります。弱起なら set_pickup({beats}) を。\n"
        ));
    }
    for line in score.text.iter().take(3) {
        let short: String = line.chars().take(160).collect();
        let ellipsis = if line.chars().count() > 160 { "…" } else { "" };
        out.push_str(&format!("メモ: {short}{ellipsis}\n"));
    }
    // A file that names its own sections is the cheapest map of itself.
    if !score.markers.is_empty() {
        out.push_str("\n区切り: ");
        let marks: Vec<String> = score
            .markers
            .iter()
            .take(24)
            .map(|(tick, name)| format!("b{} {name}", bars.position(*tick).0))
            .collect();
        out.push_str(&marks.join(" / "));
        if score.markers.len() > 24 {
            out.push_str(&format!(" …ほか{}件", score.markers.len() - 24));
        }
        out.push('\n');
    }

    out.push_str("\n番号 楽器                       音数   音域      小節     名前\n");
    for (i, track) in score.tracks.iter().enumerate().take(MAX_TRACKS) {
        let lo_key = track.notes.iter().map(|n| n.key).min().unwrap_or(0);
        let hi_key = track.notes.iter().map(|n| n.key).max().unwrap_or(0);
        let first = bars.position(track.notes.first().map_or(0, |n| n.start)).0;
        let last = bars.position(track.notes.iter().map(|n| n.start).max().unwrap_or(0)).0;
        out.push_str(&format!(
            "{:<4} {:<26} {:<6} {:<9} {:<8} {}\n",
            i,
            if track.is_percussion() {
                "パーカッション".to_string()
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
        out.push_str(&format!("…ほか{}トラック\n", score.tracks.len() - MAX_TRACKS));
    }
    out
}

/// The harmony of a passage, as chord symbols.
fn harmony(song: &Song, score: &Score, args: &Value) -> Result<String, String> {
    let bars = score.bars();
    let from = need_i64(args, "from_bar")?.max(1) as u32;
    let to = need_i64(args, "to_bar")?.max(from as i64) as u32;
    if from > bars.count {
        return Err(format!("この曲は{}小節までです。", bars.count));
    }
    // Chords are compact, so a wider window than read_bars is affordable.
    if to - from + 1 > MAX_BARS * 4 {
        return Err(format!("一度に読めるのは{}小節までです。", MAX_BARS * 4));
    }
    let tracks = wanted_tracks(score, args)?;

    let mut out = format!("{} — {from}〜{}小節の和声\n", song.title, to.min(bars.count));
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
        return "Otomo に追加の音源は入っていません（General MIDI の内蔵音色のみ）。".into();
    };
    let mut out = String::from("id                              種類        名前\n");
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
                    "トラック {bad} はありません（0〜{}）。",
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
            parts.push(format!("{label}（{}小節目〜）", span.first_bar));
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
        return Err(format!("この曲は{}小節までです（{from}小節目は範囲外）。", bars.count));
    }
    if to - from + 1 > MAX_BARS {
        return Err(format!(
            "一度に読めるのは{MAX_BARS}小節までです。{from}〜{}小節のように分けてください。",
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
    let mut out = format!("{} — {from}〜{to}小節（{} に丸め）\n", song.title, grid.label());
    for index in wanted {
        let track = &score.tracks[index];
        let lines = render_bars(score, &bars, index, from, to, velocity, grid);
        if lines.iter().all(|l| l.notes.is_empty()) {
            out.push_str(&format!("\n[{index}] {} — この範囲は休み\n", track_label(track)));
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
        if track.is_percussion() { "パーカッション" } else { program_name(track.program) };
    if track.name.is_empty() {
        instrument.to_string()
    } else {
        format!("{} · {instrument}", track.name)
    }
}

