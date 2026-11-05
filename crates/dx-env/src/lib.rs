//! `dx-env` — reproducible build-environment record.
//!
//! A `Cargo.lock` pins the dependency graph, but not the whole build
//! environment. `dx environment capture` records the toolchain, target,
//! auxiliary tools, relevant flags, the security policy/evidence and the
//! lockfile hash; `dx environment verify` recomputes the host environment and
//! reports drift. Honest scope: this is an environment *record and drift
//! check*, NOT a byte-for-byte reproducibility guarantee.

use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use std::process::Command;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tool {
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityEvidence {
    pub policy_fingerprint: String,
    pub advisory_db_snapshot: String,
    pub advisory_db_valid: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Environment {
    pub schema_version: u32,
    pub captured: String,
    pub rustc: String,
    pub host_target: String,
    pub cargo: String,
    pub rustflags: Option<String>,
    pub cargo_build_target: Option<String>,
    pub tools: Vec<Tool>,
    pub security: SecurityEvidence,
    pub lockfile_sha256: Option<String>,
}

fn first_line(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(|s| s.trim().to_string())
}

fn host_target() -> String {
    Command::new("rustc")
        .arg("-vV")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .find_map(|l| l.strip_prefix("host: ").map(str::to_string))
        })
        .unwrap_or_else(|| "unknown".into())
}

/// Capture the current environment. `generated` is an ISO-ish timestamp passed
/// by the caller (kept out of the struct's drift comparison).
pub fn capture(root: &Utf8Path, generated: &str, now_unix: u64) -> Environment {
    let policy = dx_security::Policy::from_env();
    let db_path = dx_security::default_db_path();
    let db_status = dx_security::db::evaluate(&db_path, now_unix, policy.max_db_age_secs);

    let tools = vec![
        Tool {
            name: "cargo-audit".into(),
            version: first_line("cargo", &["audit", "--version"]),
        },
        Tool {
            name: "cargo-deny".into(),
            version: first_line("cargo", &["deny", "--version"]),
        },
        Tool {
            name: "cc".into(),
            version: first_line("cc", &["--version"]),
        },
    ];

    let lockfile = root.join("Cargo.lock");
    let lockfile_sha256 = dx_integrity::sha256_file(&lockfile).ok();

    Environment {
        schema_version: SCHEMA_VERSION,
        captured: generated.to_string(),
        rustc: first_line("rustc", &["--version"]).unwrap_or_default(),
        host_target: host_target(),
        cargo: first_line("cargo", &["--version"]).unwrap_or_default(),
        rustflags: std::env::var("RUSTFLAGS").ok().filter(|s| !s.is_empty()),
        cargo_build_target: std::env::var("CARGO_BUILD_TARGET")
            .ok()
            .filter(|s| !s.is_empty()),
        tools,
        security: SecurityEvidence {
            policy_fingerprint: policy.fingerprint(),
            advisory_db_snapshot: dx_security::db::snapshot_id(&db_path),
            advisory_db_valid: db_status.valid,
        },
        lockfile_sha256,
    }
}

/// A single difference between two environments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    pub field: String,
    pub expected: String,
    pub actual: String,
    /// Critical drift fails `verify`; non-critical is informational.
    pub critical: bool,
}

fn opt(s: &Option<String>) -> String {
    s.clone().unwrap_or_else(|| "—".into())
}

/// Compare an expected (captured) environment with the actual (host) one.
/// `captured` timestamp is ignored. advisory-db snapshot/validity are reported
/// as **non-critical** (they change over time by design).
pub fn diff(expected: &Environment, actual: &Environment) -> Vec<Diff> {
    let mut d = Vec::new();
    let mut crit = |field: &str, e: String, a: String| {
        if e != a {
            d.push(Diff {
                field: field.into(),
                expected: e,
                actual: a,
                critical: true,
            });
        }
    };
    crit("rustc", expected.rustc.clone(), actual.rustc.clone());
    crit(
        "host_target",
        expected.host_target.clone(),
        actual.host_target.clone(),
    );
    crit("cargo", expected.cargo.clone(), actual.cargo.clone());
    crit(
        "rustflags",
        opt(&expected.rustflags),
        opt(&actual.rustflags),
    );
    crit(
        "cargo_build_target",
        opt(&expected.cargo_build_target),
        opt(&actual.cargo_build_target),
    );
    crit(
        "lockfile_sha256",
        opt(&expected.lockfile_sha256),
        opt(&actual.lockfile_sha256),
    );
    crit(
        "security.policy_fingerprint",
        expected.security.policy_fingerprint.clone(),
        actual.security.policy_fingerprint.clone(),
    );

    // Tools by name.
    for et in &expected.tools {
        let av = actual.tools.iter().find(|t| t.name == et.name);
        let actual_v = av
            .map(|t| opt(&t.version))
            .unwrap_or_else(|| "absent".into());
        crit(&format!("tool:{}", et.name), opt(&et.version), actual_v);
    }

    // Non-critical: advisory-db snapshot drift (informational).
    if expected.security.advisory_db_snapshot != actual.security.advisory_db_snapshot {
        d.push(Diff {
            field: "security.advisory_db_snapshot".into(),
            expected: expected.security.advisory_db_snapshot.clone(),
            actual: actual.security.advisory_db_snapshot.clone(),
            critical: false,
        });
    }
    d
}

pub fn to_json(env: &Environment) -> String {
    serde_json::to_string_pretty(env).unwrap_or_else(|_| "{}".into())
}

pub fn from_json(s: &str) -> Result<Environment, serde_json::Error> {
    serde_json::from_str(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Environment {
        Environment {
            schema_version: 1,
            captured: "t0".into(),
            rustc: "rustc 1.96.0".into(),
            host_target: "x86_64-unknown-linux-gnu".into(),
            cargo: "cargo 1.96.0".into(),
            rustflags: None,
            cargo_build_target: None,
            tools: vec![Tool {
                name: "cargo-audit".into(),
                version: Some("0.22.2".into()),
            }],
            security: SecurityEvidence {
                policy_fingerprint: "pf".into(),
                advisory_db_snapshot: "abc".into(),
                advisory_db_valid: true,
            },
            lockfile_sha256: Some("deadbeef".into()),
        }
    }

    #[test]
    fn identical_has_no_diff() {
        assert!(diff(&sample(), &sample()).is_empty());
    }

    #[test]
    fn lockfile_and_rustc_drift_are_critical() {
        let mut actual = sample();
        actual.rustc = "rustc 1.97.0".into();
        actual.lockfile_sha256 = Some("cafe".into());
        let d = diff(&sample(), &actual);
        assert_eq!(d.len(), 2);
        assert!(d.iter().all(|x| x.critical));
        assert!(d.iter().any(|x| x.field == "lockfile_sha256"));
    }

    #[test]
    fn tool_absence_is_critical_drift() {
        let mut actual = sample();
        actual.tools.clear();
        let d = diff(&sample(), &actual);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].actual, "absent");
    }

    #[test]
    fn advisory_db_snapshot_drift_is_non_critical() {
        let mut actual = sample();
        actual.security.advisory_db_snapshot = "xyz".into();
        let d = diff(&sample(), &actual);
        assert_eq!(d.len(), 1);
        assert!(!d[0].critical);
    }

    #[test]
    fn json_roundtrips() {
        let e = sample();
        let back = from_json(&to_json(&e)).unwrap();
        assert_eq!(back.rustc, e.rustc);
        assert_eq!(back.security, e.security);
    }
}
