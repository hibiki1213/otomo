//! Getting a file from the web, so that a piece which already exists is taken
//! from a source instead of being written from memory.
//!
//! Only public addresses are fetched: the URL may have come from a web page,
//! and nothing a page says should reach services on this machine or its
//! network.

use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use reqwest::Url;

/// Far above any song; small enough that a wrong link cannot fill the disk.
const MAX_BYTES: u64 = 8 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

/// What came back, sorted by what it can be used for.
pub enum Fetched {
    Midi(Vec<u8>),
    /// Plain or zipped (.mxl).
    MusicXml(Vec<u8>),
    Pdf(Vec<u8>),
    /// A photographed or scanned page: PNG or JPEG.
    Scan(Vec<u8>),
    Text(String),
    /// A web page: WebFetch reads those better than raw markup would.
    Html,
    /// A drawing in code; the text of it says nothing about the music.
    Svg,
    /// Anything else, with its content type for the message.
    Other(String),
}

pub fn get(url: &str) -> Result<Fetched, String> {
    let url = Url::parse(url.trim()).map_err(|e| format!("URL として読めません: {e}"))?;
    public(&url)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("midiss/", env!("CARGO_PKG_VERSION")))
        // Every hop is checked, or a public page could redirect inward.
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                attempt.error("リダイレクトが多すぎます")
            } else if let Err(e) = public(attempt.url()) {
                attempt.error(e)
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client.get(url.clone()).send().map_err(|e| format!("取得できませんでした: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("取得できませんでした（HTTP {status}）: {url}"));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut bytes = Vec::new();
    response
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("読み込みが途中で切れました: {e}"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("ファイルが大きすぎます（{} MB まで）。", MAX_BYTES / 1024 / 1024));
    }
    Ok(classify(bytes, &content_type))
}

/// Decide by the bytes first: servers label score files every which way.
fn classify(bytes: Vec<u8>, content_type: &str) -> Fetched {
    if bytes.starts_with(b"MThd") {
        return Fetched::Midi(bytes);
    }
    if bytes.starts_with(b"%PDF-") {
        return Fetched::Pdf(bytes);
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.starts_with(b"\xFF\xD8\xFF") {
        return Fetched::Scan(bytes);
    }
    if midiss_score::is_musicxml(&bytes) {
        return Fetched::MusicXml(bytes);
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
    if content_type.contains("html") || head.contains("<!doctype html") || head.contains("<html") {
        return Fetched::Html;
    }
    if content_type.contains("svg") || head.contains("<svg") {
        return Fetched::Svg;
    }
    if !bytes.contains(&0) {
        if let Ok(text) = String::from_utf8(bytes) {
            return Fetched::Text(text);
        }
    }
    Fetched::Other(if content_type.is_empty() { "不明な形式".into() } else { content_type.into() })
}

/// Refuse anything that is not http(s) to a public address.
fn public(url: &Url) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("{} の URL は取得できません（http / https のみ）。", url.scheme()));
    }
    let host = url.host_str().ok_or("URL にホストがありません。")?;
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs: Vec<IpAddr> = (host.trim_matches(['[', ']']), port)
        .to_socket_addrs()
        .map_err(|e| format!("{host} が見つかりません: {e}"))?
        .map(|a| a.ip())
        .collect();
    if addrs.is_empty() || addrs.iter().any(|ip| !is_public(*ip)) {
        return Err(format!("{host} はローカルネットワークのアドレスなので取得しません。"));
    }
    Ok(())
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || (a == 100 && (64..128).contains(&b))) // carrier-grade NAT
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || (first & 0xfe00) == 0xfc00 // unique local
                || (first & 0xffc0) == 0xfe80) // link-local
        }
    }
}

/// A title for a file nobody named: its file name, without the extension.
pub fn title_from(url: &str) -> Option<String> {
    let url = Url::parse(url).ok()?;
    let last = url.path_segments()?.rev().find(|s| !s.is_empty())?;
    let name = percent_decode(last);
    let stem = name.rsplit_once('.').map_or(name.as_str(), |(stem, _)| stem);
    let title = stem.replace(['_', '+'], " ").trim().to_string();
    (!title.is_empty()).then_some(title)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_addresses_are_refused() {
        for bad in [
            "http://localhost:8080/x.mid",
            "http://127.0.0.1/x.mid",
            "http://10.0.0.5/x.mid",
            "http://192.168.1.1/x.mid",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]/x.mid",
            "file:///etc/passwd",
        ] {
            assert!(public(&Url::parse(bad).unwrap()).is_err(), "{bad} should be refused");
        }
        assert!(is_public("93.184.216.34".parse().unwrap()));
        assert!(!is_public("::ffff:127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn files_are_sorted_by_their_bytes() {
        assert!(matches!(classify(b"MThd\0\0\0\x06".to_vec(), "application/octet-stream"), Fetched::Midi(_)));
        assert!(matches!(classify(b"X:1\nT:Tune\nK:G\nGABc|".to_vec(), "application/octet-stream"), Fetched::Text(_)));
        assert!(matches!(classify(b"<!DOCTYPE html><html>".to_vec(), "text/plain"), Fetched::Html));
        assert!(matches!(classify(b"PK\x03\x04\0\0".to_vec(), "application/zip"), Fetched::Other(_)));
        assert!(matches!(classify(b"%PDF-1.7\n".to_vec(), "application/octet-stream"), Fetched::Pdf(_)));
        assert!(matches!(classify(b"\xFF\xD8\xFF\xE0".to_vec(), "image/jpeg"), Fetched::Scan(_)));
        assert!(matches!(classify(b"<?xml version='1.0'?><svg/>".to_vec(), "image/svg+xml"), Fetched::Svg));
        let xml = b"<?xml version=\"1.0\"?><score-partwise><part-list/></score-partwise>";
        assert!(matches!(classify(xml.to_vec(), "text/xml"), Fetched::MusicXml(_)));
    }

    #[test]
    fn titles_come_from_file_names() {
        assert_eq!(
            title_from("https://upload.wikimedia.org/a/b/Il_Canto_degli_Italiani.mid").as_deref(),
            Some("Il Canto degli Italiani")
        );
        assert_eq!(title_from("https://example.org/Fr%C3%A8re%20Jacques.abc").as_deref(), Some("Frère Jacques"));
        assert_eq!(title_from("https://example.org/").as_deref(), None);
    }
}
