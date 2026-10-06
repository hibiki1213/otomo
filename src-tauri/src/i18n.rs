//! The display language, for what the backend says to a person directly:
//! errors the UI shows, sound-library and preset names, venue names.
//!
//! The UI decides (it follows the Mac unless the person picks one) and tells
//! us through `set_language`. What Claude reads — the chat's system prompt,
//! tool output, the MCP server — is English whatever this says.

use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

/// Whether the UI is in English (otherwise Japanese).
pub fn english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

/// `ja` or `en`, whichever the UI shows.
pub fn pick<'a>(ja: &'a str, en: &'a str) -> &'a str {
    if english() {
        en
    } else {
        ja
    }
}

/// A message in the UI's language, Japanese first:
/// `tr!("{name} が見つかりません", "{name} not found")`. Both are format
/// strings over the same arguments.
macro_rules! tr {
    ($ja:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::english() {
            format!($en $(, $arg)*)
        } else {
            format!($ja $(, $arg)*)
        }
    };
}
pub(crate) use tr;

/// Start from the Mac's language, until the UI says otherwise.
pub fn init() {
    ENGLISH.store(!preferred_language().starts_with("ja"), Ordering::Relaxed);
}

/// The person's first preferred language, as a BCP 47 tag ("ja-JP", "en-US").
#[cfg(target_os = "macos")]
fn preferred_language() -> String {
    // Read here rather than from navigator.language, which WebKit weighs
    // against the app bundle's localizations; this is the person's setting.
    objc2_foundation::NSLocale::preferredLanguages()
        .firstObject()
        .map(|s| s.to_string())
        .unwrap_or_default()
}

#[cfg(not(target_os = "macos"))]
fn preferred_language() -> String {
    std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default()
        .replace('_', "-")
}

#[tauri::command]
pub fn system_language() -> String {
    preferred_language()
}

#[tauri::command]
pub fn set_language(lang: String) {
    ENGLISH.store(lang != "ja", Ordering::Relaxed);
}
