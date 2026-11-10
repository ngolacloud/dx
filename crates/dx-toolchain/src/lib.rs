//! `dx-toolchain` — project toolchain pinning via the **native**
//! `rust-toolchain.toml`, wrapping `rustup` with provenance.
//!
//! Composes Rust's own mechanisms; it never replaces `rustup`/`rustc`
//! (ADR-0002/0020/0022). The pin DX writes is a standard `rust-toolchain.toml`,
//! so `cargo`/`rustup` honour it **without** DX ("Cargo without DX"). DX adds
//! two things on top: a **provenance record** (what the pinned channel actually
//! resolved to — rustc version/commit/host) and **drift remediation** wired
//! into `dx environment verify`.
//!
//! This crate is pure (parse/render/provenance); the `rustup` invocations live
//! in the CLI (`dx-cli::toolchaincmd`), mirroring the layering of the others.

use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use toml_edit::{Array, DocumentMut, Item, Table, Value};

/// The native file `cargo`/`rustup` read to select the toolchain.
pub const TOOLCHAIN_FILE: &str = "rust-toolchain.toml";
/// The DX provenance record of the resolved toolchain.
pub const LOCK_FILE: &str = "dx-toolchain.lock";

/// The subset of `rust-toolchain.toml` that DX manages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub channel: String,
    pub components: Vec<String>,
    pub targets: Vec<String>,
}

impl Pin {
    pub fn new(channel: impl Into<String>) -> Self {
        Self {
            channel: channel.into(),
            components: Vec::new(),
            targets: Vec::new(),
        }
    }
}

/// Provenance of a resolved toolchain: what `channel` actually resolved to.
/// Lets `dx environment verify` detect that the *same* pin now resolves to a
/// *different* rustc (e.g. a moving `stable`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// What was pinned (e.g. `1.82`, `stable`, `nightly-2026-01-01`).
    pub channel: String,
    /// Resolved `rustc --version` first line.
    pub rustc_version: String,
    pub commit_hash: Option<String>,
    pub commit_date: Option<String>,
    pub host: String,
}

/// Parse the managed subset of a `rust-toolchain.toml`.
pub fn parse_pin(s: &str) -> Option<Pin> {
    let doc = s.parse::<DocumentMut>().ok()?;
    let t = doc.get("toolchain")?.as_table_like()?;
    let channel = t.get("channel")?.as_str()?.to_string();
    let read_list = |key: &str| -> Vec<String> {
        t.get(key)
            .and_then(Item::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(Pin {
        channel,
        components: read_list("components"),
        targets: read_list("targets"),
    })
}

fn str_array(items: &[String]) -> Array {
    let mut a = Array::new();
    for it in items {
        a.push(Value::from(it.as_str()));
    }
    a
}

/// Render/update a `rust-toolchain.toml` with `pin`, **preserving** any
/// existing formatting and unmanaged keys in `existing`.
pub fn upsert_pin(existing: Option<&str>, pin: &Pin) -> String {
    let mut doc = existing
        .and_then(|s| s.parse::<DocumentMut>().ok())
        .unwrap_or_default();

    if doc.get("toolchain").and_then(Item::as_table_like).is_none() {
        doc["toolchain"] = Item::Table(Table::new());
    }
    let t = doc["toolchain"].as_table_mut().expect("toolchain table");
    t.set_implicit(false);
    t["channel"] = toml_edit::value(pin.channel.as_str());
    if pin.components.is_empty() {
        t.remove("components");
    } else {
        t["components"] = Item::Value(Value::Array(str_array(&pin.components)));
    }
    if pin.targets.is_empty() {
        t.remove("targets");
    } else {
        t["targets"] = Item::Value(Value::Array(str_array(&pin.targets)));
    }
    doc.to_string()
}

/// Parse `rustc -vV` output into a provenance record for `channel`.
pub fn parse_rustc_vv(channel: &str, vv: &str) -> Provenance {
    let mut rustc_version = String::new();
    let mut commit_hash = None;
    let mut commit_date = None;
    let mut host = String::new();
    for line in vv.lines() {
        let line = line.trim();
        if line.starts_with("rustc ") && rustc_version.is_empty() {
            rustc_version = line.to_string();
        } else if let Some(h) = line.strip_prefix("commit-hash: ") {
            commit_hash = Some(h.trim().to_string()).filter(|s| s != "unknown");
        } else if let Some(d) = line.strip_prefix("commit-date: ") {
            commit_date = Some(d.trim().to_string()).filter(|s| s != "unknown");
        } else if let Some(h) = line.strip_prefix("host: ") {
            host = h.trim().to_string();
        }
    }
    Provenance {
        channel: channel.to_string(),
        rustc_version,
        commit_hash,
        commit_date,
        host,
    }
}

pub fn provenance_to_json(p: &Provenance) -> String {
    serde_json::to_string_pretty(p).unwrap_or_else(|_| "{}".into())
}

pub fn provenance_from_json(s: &str) -> Result<Provenance, serde_json::Error> {
    serde_json::from_str(s)
}

/// Read the project pin, if `rust-toolchain.toml` exists and is well-formed.
pub fn read_pin(root: &Utf8Path) -> Option<Pin> {
    let text = std::fs::read_to_string(root.join(TOOLCHAIN_FILE).as_std_path()).ok()?;
    parse_pin(&text)
}

/// Read the recorded provenance, if present.
pub fn read_provenance(root: &Utf8Path) -> Option<Provenance> {
    let text = std::fs::read_to_string(root.join(LOCK_FILE).as_std_path()).ok()?;
    provenance_from_json(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VV: &str = "rustc 1.82.0 (f6e511eec 2024-10-15)\n\
binary: rustc\n\
commit-hash: f6e511eec7342f59a25f7c0534f1dbea00d01b14\n\
commit-date: 2024-10-15\n\
host: x86_64-unknown-linux-gnu\n\
release: 1.82.0\n\
LLVM version: 19.1.1\n";

    #[test]
    fn parse_vv_extracts_fields() {
        let p = parse_rustc_vv("1.82", VV);
        assert_eq!(p.channel, "1.82");
        assert_eq!(p.rustc_version, "rustc 1.82.0 (f6e511eec 2024-10-15)");
        assert_eq!(
            p.commit_hash.as_deref(),
            Some("f6e511eec7342f59a25f7c0534f1dbea00d01b14")
        );
        assert_eq!(p.commit_date.as_deref(), Some("2024-10-15"));
        assert_eq!(p.host, "x86_64-unknown-linux-gnu");
    }

    #[test]
    fn vv_unknown_commit_is_none() {
        let vv = "rustc 1.82.0\ncommit-hash: unknown\nhost: x86_64-unknown-linux-gnu\n";
        let p = parse_rustc_vv("stable", vv);
        assert!(p.commit_hash.is_none());
    }

    #[test]
    fn upsert_creates_fresh_pin() {
        let out = upsert_pin(None, &Pin::new("1.82"));
        let back = parse_pin(&out).unwrap();
        assert_eq!(back.channel, "1.82");
        assert!(back.components.is_empty());
    }

    #[test]
    fn upsert_updates_channel_preserving_other_keys() {
        let existing = "[toolchain]\nchannel = \"1.80\"\nprofile = \"minimal\"\n";
        let out = upsert_pin(Some(existing), &Pin::new("1.82"));
        assert!(out.contains("channel = \"1.82\""));
        // Unmanaged key survives.
        assert!(out.contains("profile = \"minimal\""));
    }

    #[test]
    fn upsert_roundtrips_components_and_targets() {
        let pin = Pin {
            channel: "stable".into(),
            components: vec!["clippy".into(), "rustfmt".into()],
            targets: vec!["wasm32-unknown-unknown".into()],
        };
        let out = upsert_pin(None, &pin);
        assert_eq!(parse_pin(&out).unwrap(), pin);
    }

    #[test]
    fn provenance_json_roundtrips() {
        let p = parse_rustc_vv("1.82", VV);
        assert_eq!(provenance_from_json(&provenance_to_json(&p)).unwrap(), p);
    }

    #[test]
    fn parse_pin_requires_channel() {
        assert!(parse_pin("[toolchain]\nprofile = \"minimal\"\n").is_none());
    }
}
