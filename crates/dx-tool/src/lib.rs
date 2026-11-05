//! `dx-tool` — project-pinned developer tools.
//!
//! Tools are NOT libraries: `dx add` manages `[dependencies]`; `dx tool`
//! manages executables pinned per project in `dx-tools.toml`, installed into a
//! project-local directory and recorded with **provenance** (source, version
//! and binary sha256) in a lock. `dx tool run` uses ONLY the project-pinned
//! tool; it never falls back to a system binary of uncertain provenance.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where a tool comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolSource {
    /// crates.io or an alternative registry.
    Registry { registry: Option<String> },
    /// Local path crate (for controlled/offline installs).
    Path { path: String },
}

impl Default for ToolSource {
    fn default() -> Self {
        ToolSource::Registry { registry: None }
    }
}

/// A pinned tool as declared in `dx-tools.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSpec {
    pub name: String,
    pub version: Option<String>,
    pub source: ToolSource,
}

// --- dx-tools.toml ---------------------------------------------------------

#[derive(Deserialize, Default)]
struct ToolsDoc {
    #[serde(default)]
    tools: BTreeMap<String, RawTool>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawTool {
    Version(String),
    Full {
        version: Option<String>,
        registry: Option<String>,
        path: Option<String>,
    },
}

/// Parse `dx-tools.toml` into specs (sorted by name).
pub fn parse_manifest(text: &str) -> Result<Vec<ToolSpec>, toml::de::Error> {
    let doc: ToolsDoc = toml::from_str(text)?;
    Ok(doc
        .tools
        .into_iter()
        .map(|(name, raw)| match raw {
            RawTool::Version(v) => ToolSpec {
                name,
                version: Some(v),
                source: ToolSource::Registry { registry: None },
            },
            RawTool::Full {
                version,
                registry,
                path,
            } => {
                let source = match path {
                    Some(p) => ToolSource::Path { path: p },
                    None => ToolSource::Registry { registry },
                };
                ToolSpec {
                    name,
                    version,
                    source,
                }
            }
        })
        .collect())
}

/// Render specs back to a deterministic `dx-tools.toml` body.
pub fn render_manifest(specs: &[ToolSpec]) -> String {
    let mut out = String::from("[tools]\n");
    let mut sorted = specs.to_vec();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    for s in &sorted {
        match (&s.source, &s.version) {
            (ToolSource::Registry { registry: None }, Some(v)) => {
                out.push_str(&format!("{} = \"{}\"\n", s.name, v));
            }
            (ToolSource::Registry { registry }, v) => {
                out.push_str(&format!(
                    "{}.version = \"{}\"\n",
                    s.name,
                    v.clone().unwrap_or_default()
                ));
                if let Some(r) = registry {
                    out.push_str(&format!("{}.registry = \"{}\"\n", s.name, r));
                }
            }
            (ToolSource::Path { path }, _) => {
                out.push_str(&format!("{}.path = \"{}\"\n", s.name, path));
            }
        }
    }
    out
}

/// Insert/replace a spec by name (idempotent); returns `true` if it changed.
pub fn upsert(specs: &mut Vec<ToolSpec>, spec: ToolSpec) -> bool {
    if let Some(existing) = specs.iter_mut().find(|s| s.name == spec.name) {
        if *existing == spec {
            return false;
        }
        *existing = spec;
        true
    } else {
        specs.push(spec);
        true
    }
}

// --- provenance lock -------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledTool {
    pub name: String,
    pub version: String,
    /// Human-readable source (e.g. "registry:crates.io", "path:../x").
    pub source: String,
    /// sha256 of the installed binary (provenance / tamper check).
    pub sha256: String,
    pub binary: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolLock {
    pub schema_version: u32,
    pub installed: Vec<InstalledTool>,
}

impl ToolLock {
    pub fn find(&self, name: &str) -> Option<&InstalledTool> {
        self.installed.iter().find(|t| t.name == name)
    }
    pub fn upsert(&mut self, t: InstalledTool) {
        if let Some(e) = self.installed.iter_mut().find(|x| x.name == t.name) {
            *e = t;
        } else {
            self.installed.push(t);
        }
        self.installed.sort_by(|a, b| a.name.cmp(&b.name));
    }
}

pub fn parse_lock(text: &str) -> Result<ToolLock, serde_json::Error> {
    serde_json::from_str(text)
}
pub fn render_lock(lock: &ToolLock) -> String {
    serde_json::to_string_pretty(lock).unwrap_or_else(|_| "{}".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_short_and_full_forms() {
        let toml = r#"
[tools]
cargo-nextest = "0.9.100"
mdbook = { version = "0.4.40", registry = "empresa" }
local-fmt = { path = "../local-fmt" }
"#;
        let specs = parse_manifest(toml).unwrap();
        assert_eq!(specs.len(), 3);
        let nx = specs.iter().find(|s| s.name == "cargo-nextest").unwrap();
        assert_eq!(nx.version.as_deref(), Some("0.9.100"));
        let mb = specs.iter().find(|s| s.name == "mdbook").unwrap();
        assert_eq!(
            mb.source,
            ToolSource::Registry {
                registry: Some("empresa".into())
            }
        );
        let lf = specs.iter().find(|s| s.name == "local-fmt").unwrap();
        assert_eq!(
            lf.source,
            ToolSource::Path {
                path: "../local-fmt".into()
            }
        );
    }

    #[test]
    fn upsert_is_idempotent() {
        let mut specs = vec![ToolSpec {
            name: "t".into(),
            version: Some("1".into()),
            source: ToolSource::default(),
        }];
        let same = ToolSpec {
            name: "t".into(),
            version: Some("1".into()),
            source: ToolSource::default(),
        };
        assert!(!upsert(&mut specs, same));
        let changed = ToolSpec {
            name: "t".into(),
            version: Some("2".into()),
            source: ToolSource::default(),
        };
        assert!(upsert(&mut specs, changed));
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].version.as_deref(), Some("2"));
    }

    #[test]
    fn lock_roundtrip_and_find() {
        let mut lock = ToolLock {
            schema_version: 1,
            installed: vec![],
        };
        lock.upsert(InstalledTool {
            name: "t".into(),
            version: "1.0.0".into(),
            source: "registry:crates.io".into(),
            sha256: "abc".into(),
            binary: ".dx/tools/bin/t".into(),
        });
        let back = parse_lock(&render_lock(&lock)).unwrap();
        assert_eq!(back.find("t").unwrap().sha256, "abc");
    }

    #[test]
    fn render_manifest_is_deterministic() {
        let specs = vec![
            ToolSpec {
                name: "b".into(),
                version: Some("2".into()),
                source: ToolSource::default(),
            },
            ToolSpec {
                name: "a".into(),
                version: Some("1".into()),
                source: ToolSource::default(),
            },
        ];
        let out = render_manifest(&specs);
        assert!(out.find("a = ").unwrap() < out.find("b = ").unwrap());
    }
}
