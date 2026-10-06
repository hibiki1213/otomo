//! Sheet music as a PDF: the pages `engrave()` (src/lib/engrave.ts) lays out
//! as SVG become vector PDF pages, one each.
//!
//! usvg reads the SVG and lays out its text; krilla writes the PDF, with the
//! fonts subset and the text left selectable. usvg does not read CSS
//! `@font-face`, and Verovio embeds its music font that way for the glyphs it
//! sets inside text (the note in "♩ = 120"), so those fonts are loaded here.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use base64::Engine as _;
use krilla::geom::Size;
use krilla::page::PageSettings;
use krilla::Document;
use krilla_svg::{SurfaceExt, SvgSettings};
use usvg::fontdb::{self, Database, Family, Query, ID};

use crate::i18n::tr;

const PT_PER_MM: f64 = 72.0 / 25.4;

/// Japanese fonts to set kana and kanji in, Mincho first to sit with
/// Verovio's Times. Whichever the system has.
const CJK_FALLBACK: &[&str] = &[
    "Hiragino Mincho ProN",
    "Yu Mincho",
    "Noto Serif CJK JP",
    "Hiragino Sans",
    "Yu Gothic",
    "Noto Sans CJK JP",
];

/// Writes `pages` (SVG, each covering the whole sheet) to `path` as a PDF
/// with `width_mm` × `height_mm` pages.
#[tauri::command]
pub async fn pdf_write(path: String, pages: Vec<String>, width_mm: f64, height_mm: f64) -> Result<(), String> {
    // Laying out text and subsetting fonts takes a while for a long score.
    tauri::async_runtime::spawn_blocking(move || {
        let pdf = render(&pages, width_mm, height_mm)?;
        std::fs::write(&path, pdf).map_err(|e| tr!("PDF を保存できませんでした: {e}", "Couldn't save the PDF: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

fn render(pages: &[String], width_mm: f64, height_mm: f64) -> Result<Vec<u8>, String> {
    if pages.is_empty() {
        return Err(tr!("書き出すページがありません", "There are no pages to write"));
    }
    let size = Size::from_wh((width_mm * PT_PER_MM) as f32, (height_mm * PT_PER_MM) as f32)
        .ok_or_else(|| tr!("用紙サイズが正しくありません", "The paper size is invalid"))?;
    let options = svg_options(pages);

    let mut document = Document::new();
    for (i, svg) in pages.iter().enumerate() {
        let n = i + 1;
        let tree = usvg::Tree::from_str(&strip_titles(svg), &options)
            .map_err(|e| tr!("{n} ページ目を読めませんでした: {e}", "Couldn't read page {n}: {e}"))?;
        let mut page = document.start_page_with(PageSettings::new(size));
        let mut surface = page.surface();
        surface.draw_svg(&tree, size, SvgSettings::default());
        surface.finish();
        page.finish();
    }
    document
        .finish()
        .map_err(|e| tr!("PDF を作れませんでした: {e}", "Couldn't make the PDF: {e}"))
}

fn svg_options(pages: &[String]) -> usvg::Options<'static> {
    usvg::Options {
        fontdb: fonts(pages),
        font_resolver: usvg::FontResolver {
            select_font: usvg::FontResolver::default_font_selector(),
            select_fallback: fallback(),
        },
        ..Default::default()
    }
}

/// Drops `<title>` elements, which are never drawn. Inside `<text>`, usvg
/// counts one toward where a `<tspan>` starts and loses the tspan's x and y,
/// and Verovio's title block is written that way.
fn strip_titles(svg: &str) -> Cow<'_, str> {
    if !svg.contains("<title") {
        return Cow::Borrowed(svg);
    }
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(start) = rest.find("<title") {
        let after = &rest[start + "<title".len()..];
        if !after.starts_with(['>', '/', ' ', '\t', '\n', '\r']) {
            // Some other element whose name starts with "title".
            out.push_str(&rest[..rest.len() - after.len()]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..start]);
        let empty = after.find('>').is_some_and(|gt| after[..gt].ends_with('/'));
        rest = match after.find(if empty { "/>" } else { "</title>" }) {
            Some(end) => &after[end + if empty { 2 } else { 8 }..],
            None => "",
        };
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// The system fonts, scanned once: there are hundreds of them.
fn system_fonts() -> Arc<Database> {
    static FONTS: OnceLock<Arc<Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

/// The system fonts, plus those the pages embed with `@font-face`.
fn fonts(pages: &[String]) -> Arc<Database> {
    let mut db = system_fonts();
    let mut seen = HashSet::new();
    for (family, data) in pages.iter().flat_map(|svg| font_faces(svg)) {
        if !seen.insert(family.clone()) {
            continue;
        }
        match sfnt(data) {
            Some(data) => add_font(Arc::make_mut(&mut db), &family, data),
            None => log::warn!("pdf: can't read the embedded font {family}"),
        }
    }
    db
}

/// `(family, bytes)` of each `@font-face` whose source is a base64 `data:` URL.
fn font_faces(svg: &str) -> Vec<(String, Vec<u8>)> {
    let mut faces = Vec::new();
    for rule in svg.split("@font-face").skip(1) {
        let rule = &rule[..rule.find('}').unwrap_or(rule.len())];
        let family = rule
            .split("font-family:")
            .nth(1)
            .and_then(|s| s.split(';').next())
            .map(|s| s.trim().trim_matches(['\'', '"']).to_owned());
        let data = rule
            .split(";base64,")
            .nth(1)
            .and_then(|s| s.split([')', '\'', '"']).next())
            .map(|s| s.split_whitespace().collect::<String>());
        if let (Some(family), Some(data)) = (family, data) {
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) {
                faces.push((family, bytes));
            }
        }
    }
    faces
}

/// The font as TrueType/OpenType, which is all fontdb reads.
fn sfnt(data: Vec<u8>) -> Option<Vec<u8>> {
    match data.get(..4)? {
        b"wOF2" => wuff::decompress_woff2(&data).ok(),
        b"wOFF" => None,
        _ => Some(data),
    }
}

/// Text asks for the font by its `@font-face` name, which need not be the
/// one inside the font, so the face answers to both.
fn add_font(db: &mut Database, family: &str, data: Vec<u8>) {
    for id in db.load_font_source(fontdb::Source::Binary(Arc::new(data))) {
        let Some(face) = db.face(id) else { continue };
        if face.families.iter().any(|(name, _)| name.eq_ignore_ascii_case(family)) {
            continue;
        }
        let mut face = face.clone();
        face.families.insert(0, (family.to_owned(), fontdb::Language::English_UnitedStates));
        db.remove_face(id);
        db.push_face_info(face);
    }
}

/// For characters the font has not got. Japanese ones come from a Japanese
/// font of the same weight: usvg's own fallback takes the first font in the
/// database with the character, which may be a Chinese one, or a different
/// one for kana and kanji.
///
/// usvg shapes a whole line with each span's font, then with the fallback,
/// and keeps the fallback outright when it has every character. So the
/// SMuFL glyph in "♩ = 120", which only the music font span can set, must not
/// find a fallback: private-use characters mean something else in every
/// other font that has them, and Apple's LastResort "has" them all as boxes.
fn fallback() -> usvg::FallbackSelectionFn<'static> {
    Box::new(|c, used, db| {
        if matches!(c, '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..) {
            return None;
        }
        let usable = |id: ID| !used.contains(&id) && has_char(db, id, c);
        let base = used.first().and_then(|id| db.face(*id));
        let (weight, style) = base.map_or((fontdb::Weight::NORMAL, fontdb::Style::Normal), |f| (f.weight, f.style));
        let japanese = CJK_FALLBACK.iter().find_map(|name| {
            let families = [Family::Name(name)];
            db.query(&Query { families: &families, weight, style, ..Query::default() }).filter(|&id| usable(id))
        });
        japanese.or_else(|| {
            db.faces()
                .filter(|face| !face.post_script_name.contains("LastResort"))
                .map(|face| face.id)
                .find(|&id| usable(id))
        })
    })
}

fn has_char(db: &Database, id: ID, c: char) -> bool {
    db.with_face_data(id, |data, index| {
        ttf_parser::Face::parse(data, index).ok()?.glyph_index(c)
    })
    .flatten()
    .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(body: &str) -> String {
        format!(
            r#"<svg width="2100px" height="2970px" xmlns="http://www.w3.org/2000/svg">
<svg class="definition-scale" font-family="Times, serif" viewBox="0 0 21000 29700">{body}</svg></svg>"#
        )
    }

    #[test]
    fn reads_font_face_data_urls() {
        let data = base64::engine::general_purpose::STANDARD.encode(b"\0\x01\0\0font");
        let svg = format!(
            "<style type=\"text/css\">@font-face {{\n    font-family: 'Leipzig';\n    src: url(data:application/font-woff2;charset=utf-8;base64,{data}) format('woff2');\n}}</style>"
        );
        assert_eq!(font_faces(&svg), vec![("Leipzig".to_owned(), b"\0\x01\0\0font".to_vec())]);
        assert!(font_faces("<svg/>").is_empty());
    }

    #[test]
    fn writes_one_pdf_page_per_svg() {
        let pages = [
            page(r#"<text x="10500" y="1000" text-anchor="middle" font-size="607px">ふるさと Title</text><path d="M1000 3000 L20000 3000" stroke="black" stroke-width="20"/>"#),
            page(r#"<text x="10500" y="29000" text-anchor="middle" font-size="350px">2</text>"#),
        ]
        .to_vec();
        let pdf = render(&pages, 210.0, 297.0).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        let text = String::from_utf8_lossy(&pdf);
        assert_eq!(text.matches("/Type/Page/").count(), 2);
        assert!(text.contains("/MediaBox[0 0 595.2756 841.8898]"));
        // Kana come from a Japanese font, not whichever font has them first.
        #[cfg(target_os = "macos")]
        assert!(text.contains("HiraMinProN"));
    }

    #[test]
    fn strips_titles() {
        let svg = r#"<text><tspan x="1"> <title class="labelAttr">title</title><tspan>ふ</tspan></tspan><title/><titled/></text>"#;
        assert_eq!(strip_titles(svg), r#"<text><tspan x="1"> <tspan>ふ</tspan></tspan><titled/></text>"#);
        assert!(matches!(strip_titles("<svg/>"), Cow::Borrowed(_)));
    }

    #[test]
    fn no_fallback_for_private_use() {
        let mut db = system_fonts();
        let first = db.faces().next().map(|f| f.id);
        if let Some(id) = first {
            assert_eq!(fallback()('\u{ECA5}', &[id], &mut db), None);
        }
    }

    #[test]
    fn rejects_no_pages() {
        assert!(render(&[], 210.0, 297.0).is_err());
    }
}
