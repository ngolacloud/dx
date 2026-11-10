//! Per-crate advisory **fast-BLOCK** pre-screen (ADR-0026).
//!
//! The global cache (`global.rs`) reuses a verdict only for the identical
//! `Cargo.lock`. This module adds cross-project value **without weakening the
//! gate**: it remembers, per advisory-db snapshot, which `crate@version`s
//! carried a blocking advisory, so a *different* project pulling a
//! known-vulnerable crate is **blocked immediately** — without running the full
//! gate.
//!
//! ## Why this is sound (and never shortcuts to Approved)
//!
//! `cargo-audit` findings (RUSTSEC advisories: vulnerability/unmaintained/
//! yanked) are a property of a `crate@version` under a given advisory-db
//! snapshot — independent of the rest of the graph. So "this crate is blocked"
//! transfers soundly between projects. The **opposite** does not: `cargo-deny`
//! has graph-level checks (duplicate versions, wildcards) that do NOT decompose
//! per crate. Therefore this pre-screen may only ever reach a **Blocked**
//! decision; it NEVER returns Approved. The Approved path always runs the full
//! gate (audit + deny, incl. graph-level checks). See ADR-0005/0026.
//!
//! Only `cargo-audit` categories are cached (the per-crate-sound set); deny
//! categories are deliberately excluded.

use camino::{Utf8Path, Utf8PathBuf};
use std::collections::BTreeMap;

use crate::audit::Finding;
use crate::policy::{Action, Category, Policy};

fn home() -> Utf8PathBuf {
    Utf8PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

/// Directory of the per-crate advisory cache (`DX_ADVISORY_DIR` or
/// `~/.dx/advisories`).
fn advisory_dir() -> Utf8PathBuf {
    match std::env::var("DX_ADVISORY_DIR") {
        Ok(d) if !d.is_empty() => Utf8PathBuf::from(d),
        _ => home().join(".dx/advisories"),
    }
}

/// Only these (cargo-audit) categories are per-crate-sound to cache.
fn is_per_crate_sound(cat: Category) -> bool {
    matches!(
        cat,
        Category::Vulnerability | Category::Unmaintained | Category::Yanked | Category::Malware
    )
}

/// Sanitize a snapshot id into a filename.
fn snapshot_file(snapshot: &str) -> Utf8PathBuf {
    let safe: String = snapshot
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    advisory_dir().join(format!("{safe}.json"))
}

type Index = BTreeMap<String, Vec<Finding>>;

fn load_index(snapshot: &str) -> Index {
    std::fs::read_to_string(snapshot_file(snapshot).as_std_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Record the per-crate advisory findings of a full gate run, so future
/// pre-screens can fast-block the same `crate@version` under this snapshot.
/// Only per-crate-sound categories are stored. "Clean" is never recorded.
pub fn record(snapshot: &str, findings: &[Finding]) {
    let relevant: Vec<&Finding> = findings
        .iter()
        .filter(|f| is_per_crate_sound(f.category))
        .collect();
    if relevant.is_empty() {
        return;
    }
    let mut index = load_index(snapshot);
    for f in relevant {
        let key = format!("{}@{}", f.package, f.version);
        let entry = index.entry(key).or_default();
        // De-dup by advisory id (or title when id is absent).
        let id = f.advisory_id.clone().unwrap_or_else(|| f.title.clone());
        let known = entry
            .iter()
            .any(|e| e.advisory_id.clone().unwrap_or_else(|| e.title.clone()) == id);
        if !known {
            entry.push(f.clone());
        }
    }
    let dir = advisory_dir();
    if std::fs::create_dir_all(dir.as_std_path()).is_err() {
        return;
    }
    if let Ok(data) = serde_json::to_string_pretty(&index) {
        let _ = std::fs::write(snapshot_file(snapshot).as_std_path(), data);
    }
}

/// Parse `crate@version` pairs out of a `Cargo.lock` (line-based; the `[[package]]`
/// format is stable). Avoids pulling in a TOML dependency here.
pub fn packages_in_lock(lockfile: &Utf8Path) -> Vec<(String, String)> {
    let text = match std::fs::read_to_string(lockfile.as_std_path()) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    let mut in_pkg = false;
    let unquote = |line: &str, key: &str| -> Option<String> {
        let rest = line.trim().strip_prefix(key)?.trim_start();
        let rest = rest.strip_prefix('=')?.trim();
        Some(rest.trim_matches('"').to_string())
    };
    for line in text.lines() {
        let t = line.trim();
        if t == "[[package]]" {
            in_pkg = true;
            name = None;
            continue;
        }
        if t.starts_with('[') && t != "[[package]]" {
            in_pkg = false;
            continue;
        }
        if !in_pkg {
            continue;
        }
        if let Some(n) = unquote(t, "name") {
            name = Some(n);
        } else if let Some(v) = unquote(t, "version") {
            if let Some(n) = name.take() {
                out.push((n, v));
            }
        }
    }
    out
}

/// Pre-screen the current lock against the per-crate cache. Returns the blocking
/// findings if **any** crate in the lock is already known-blocked under the
/// current snapshot AND policy; otherwise `None` (the caller runs the full gate).
///
/// Never returns a non-blocking result — this can only ever accelerate a BLOCK.
pub fn screen(snapshot: &str, policy: &Policy, lockfile: &Utf8Path) -> Option<Vec<Finding>> {
    let index = load_index(snapshot);
    if index.is_empty() {
        return None;
    }
    let packages = packages_in_lock(lockfile);
    let mut blocking: Vec<Finding> = Vec::new();
    for (name, version) in packages {
        let key = format!("{name}@{version}");
        if let Some(findings) = index.get(&key) {
            for f in findings {
                if policy.action_for(f.category) == Action::Block {
                    blocking.push(f.clone());
                }
            }
        }
    }
    if blocking.is_empty() {
        None
    } else {
        Some(blocking)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn vuln(pkg: &str, ver: &str) -> Finding {
        Finding {
            category: Category::Vulnerability,
            package: pkg.into(),
            version: ver.into(),
            advisory_id: Some("RUSTSEC-2021-0003".into()),
            title: "Buffer overflow".into(),
            url: None,
            dependency_path: None,
        }
    }

    fn with_dir(dir: &Utf8Path, f: impl FnOnce()) {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("DX_ADVISORY_DIR", dir.as_str());
        f();
        std::env::remove_var("DX_ADVISORY_DIR");
    }

    fn write_lock(dir: &Utf8Path, pkgs: &[(&str, &str)]) -> Utf8PathBuf {
        let mut s = String::from("version = 3\n");
        for (n, v) in pkgs {
            s.push_str(&format!(
                "\n[[package]]\nname = \"{n}\"\nversion = \"{v}\"\n"
            ));
        }
        let path = dir.join("Cargo.lock");
        std::fs::write(path.as_std_path(), s).unwrap();
        path
    }

    #[test]
    fn parses_lock_packages() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        let lock = write_lock(dir, &[("serde", "1.0.200"), ("smallvec", "1.6.0")]);
        let pkgs = packages_in_lock(&lock);
        assert!(pkgs.contains(&("serde".into(), "1.0.200".into())));
        assert!(pkgs.contains(&("smallvec".into(), "1.6.0".into())));
    }

    #[test]
    fn record_then_screen_fast_blocks_known_bad_crate() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        with_dir(dir, || {
            record("snapA", &[vuln("smallvec", "1.6.0")]);
            // A DIFFERENT project that pulls the same bad crate:
            let lock = write_lock(dir, &[("serde", "1.0.200"), ("smallvec", "1.6.0")]);
            let hit = screen("snapA", &Policy::default(), &lock);
            assert!(hit.is_some());
            assert_eq!(hit.unwrap()[0].package, "smallvec");
        });
    }

    #[test]
    fn screen_misses_when_crate_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        with_dir(dir, || {
            record("snapA", &[vuln("smallvec", "1.6.0")]);
            let lock = write_lock(dir, &[("serde", "1.0.200")]);
            assert!(screen("snapA", &Policy::default(), &lock).is_none());
        });
    }

    #[test]
    fn screen_misses_on_different_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        with_dir(dir, || {
            record("snapA", &[vuln("smallvec", "1.6.0")]);
            let lock = write_lock(dir, &[("smallvec", "1.6.0")]);
            assert!(screen("snapB", &Policy::default(), &lock).is_none());
        });
    }

    #[test]
    fn deny_categories_are_not_cached() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        with_dir(dir, || {
            let license = Finding {
                category: Category::License,
                package: "x".into(),
                version: "1".into(),
                advisory_id: None,
                title: "license".into(),
                url: None,
                dependency_path: None,
            };
            record("snapA", &[license]);
            let lock = write_lock(dir, &[("x", "1")]);
            // License is graph/deny-adjacent — never pre-screened.
            assert!(screen("snapA", &Policy::default(), &lock).is_none());
        });
    }

    #[test]
    fn unmaintained_does_not_block_under_default_policy() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        with_dir(dir, || {
            let unmaint = Finding {
                category: Category::Unmaintained,
                package: "old".into(),
                version: "0.1.0".into(),
                advisory_id: Some("RUSTSEC-2020-0000".into()),
                title: "unmaintained".into(),
                url: None,
                dependency_path: None,
            };
            record("snapA", &[unmaint]);
            let lock = write_lock(dir, &[("old", "0.1.0")]);
            // Default policy warns (not blocks) on unmaintained ⇒ no fast-block.
            assert!(screen("snapA", &Policy::default(), &lock).is_none());
        });
    }
}
