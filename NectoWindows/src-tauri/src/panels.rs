//
//  Copyright (c) 2026 Viva Republica, Inc.
//

//! Panels an app carries: fetched as an archive, checked against the registered stamp,
//! kept in memory and served through the `necto-plugin` scheme.

use base64::Engine;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub struct Panel {
    pub manifest: Value,
    files: BTreeMap<String, Vec<u8>>,
}

impl Panel {
    /// Decodes `{"files": [{"path", "data": base64}]}` and checks it against the stamp the
    /// app registered: `contentHash` when present, the legacy `hash` otherwise.
    pub fn decode(archive: &Value, stamp: &Value) -> Result<Panel, String> {
        let rows = archive["files"].as_array().ok_or("the archive has no files")?;
        let mut files = BTreeMap::new();
        for row in rows {
            let (Some(path), Some(data)) = (row["path"].as_str(), row["data"].as_str()) else {
                return Err("an archive entry has no path or data".into());
            };
            let data = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| e.to_string())?;
            files.insert(path.to_string(), data);
        }
        let matches = match stamp["contentHash"].as_str() {
            Some(expected) => content_hash(&files) == expected,
            None => stamp["hash"].as_str() == Some(&legacy_hash(&files)),
        };
        if !matches {
            return Err("the archive does not match the stamp the app registered".into());
        }
        let manifest = files.get("manifest.json").ok_or("the archive has no manifest.json")?;
        let manifest: Value = serde_json::from_slice(manifest).map_err(|e| e.to_string())?;
        Ok(Panel { manifest, files })
    }

    pub fn name(&self) -> &str {
        self.manifest["name"].as_str().or(self.manifest["id"].as_str()).unwrap_or_default()
    }

    pub fn operation(&self, id: &str) -> Option<&Value> {
        self.manifest["operations"].as_array()?.iter().find(|op| op["id"] == id)
    }

    pub fn operations(&self) -> impl Iterator<Item = &Value> {
        self.manifest["operations"].as_array().into_iter().flatten()
    }

    /// A file's bytes and media type. `index.html` gets the bridge shim before any other script.
    pub fn file(&self, path: &str) -> Option<(Vec<u8>, &'static str)> {
        let path = if path.is_empty() { "index.html" } else { path };
        let data = self.files.get(path)?;
        let mime = media_type(path);
        if path != "index.html" {
            return Some((data.clone(), mime));
        }
        let html = String::from_utf8_lossy(data);
        let script = format!("<script>{SHIM}</script>");
        let at = html.to_ascii_lowercase().find("<head>").map_or(0, |i| i + "<head>".len());
        Some(([&html[..at], &script, &html[at..]].concat().into_bytes(), mime))
    }
}

/// Stands in for WebKit's message handler: requests go to the shell, replies and stream
/// messages come back from it. Panels and `@necto/bridge` run unchanged.
const SHIM: &str = r#"(() => {
  let next = 0;
  const waiting = new Map();
  document.documentElement.lang = navigator.language;
  window.webkit = { messageHandlers: { necto: { postMessage(message) {
    const id = ++next;
    return new Promise((resolve) => { waiting.set(id, resolve); parent.postMessage({ nectoRequest: id, message }, "*"); });
  } } } };
  addEventListener("message", (event) => {
    if (event.source !== parent || !event.data) return;
    const data = event.data;
    if (data.nectoReply) { const resolve = waiting.get(data.nectoReply); waiting.delete(data.nectoReply); resolve?.(data.reply); }
    else if (data.nectoStream === "event") window.__nectoBridgeDeliver?.(data.subscriptionID, data.payload);
    else if (data.nectoStream === "error") window.__nectoBridgeStreamError?.(data.subscriptionID, data.payload);
    else if (data.nectoStream === "end") window.__nectoBridgeStreamEnd?.(data.subscriptionID);
  });
})();"#;

fn length(digest: &mut Sha256, n: usize) {
    digest.update((n as u64).to_be_bytes());
}

/// Matches `NectoPanelArchive.contentHash`: counts and lengths as big endian u64.
pub fn content_hash(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut digest = Sha256::new();
    length(&mut digest, files.len());
    for (path, data) in files {
        length(&mut digest, path.len());
        digest.update(path.as_bytes());
        length(&mut digest, data.len());
        digest.update(data);
    }
    hex(digest)
}

/// Matches `NectoPanelArchive.legacyContentHash`, which older SDKs still send.
pub fn legacy_hash(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut digest = Sha256::new();
    for (path, data) in files {
        digest.update(path.as_bytes());
        digest.update([0]);
        digest.update(data);
        digest.update([0]);
    }
    hex(digest)
}

fn hex(digest: Sha256) -> String {
    digest.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

pub fn media_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn archive() -> (Value, BTreeMap<String, Vec<u8>>) {
        let files = BTreeMap::from([
            ("index.html".to_string(), b"<html><head><title>x</title></head></html>".to_vec()),
            ("manifest.json".to_string(), br#"{"id":"p","name":"P","operations":[{"id":"a"}]}"#.to_vec()),
        ]);
        let engine = base64::engine::general_purpose::STANDARD;
        let rows: Vec<Value> = files.iter().map(|(p, d)| json!({ "path": p, "data": engine.encode(d) })).collect();
        (json!({ "files": rows }), files)
    }

    #[test]
    fn checks_the_stamp_and_injects_the_shim() {
        let (value, files) = archive();
        assert!(Panel::decode(&value, &json!({ "hash": "nope", "contentHash": content_hash(&files) })).is_ok());
        assert!(Panel::decode(&value, &json!({ "hash": legacy_hash(&files) })).is_ok());
        assert!(Panel::decode(&value, &json!({ "hash": legacy_hash(&files), "contentHash": "bad" })).is_err());

        let panel = Panel::decode(&value, &json!({ "hash": legacy_hash(&files) })).unwrap();
        assert_eq!(panel.name(), "P");
        assert!(panel.operation("a").is_some());
        let (html, mime) = panel.file("").unwrap();
        let html = String::from_utf8(html).unwrap();
        assert!(html.starts_with("<html><head><script>(() => {"));
        assert!(mime.starts_with("text/html"));
        assert!(panel.file("missing.js").is_none());
    }
}
