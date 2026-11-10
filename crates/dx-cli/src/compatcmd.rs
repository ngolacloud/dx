//! `dx compat` — probe the host Cargo against the compatibility contract.
//!
//! DX delegates to a set of stable `cargo`/`rustc` interfaces (ADR-0025). This
//! command checks the host Cargo version against the tested range and probes
//! each required interface for availability/format, so coupling breakage is
//! caught explicitly rather than at some random downstream failure. Exits
//! non-zero if a REQUIRED interface is broken.

use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context, Result};

use dx_compat::{classify, Support};

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
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

/// A subcommand is "available" if `cargo <sub> --help` exits 0.
fn subcommand_available(sub: &str) -> bool {
    Command::new(cargo())
        .args([sub, "--help"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check `cargo metadata --format-version 1` really yields format 1 JSON.
/// Returns `Some(true/false)` when run in a project, `None` when not in one.
fn metadata_format_ok() -> Option<bool> {
    let out = Command::new(cargo())
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .ok()?;
    if !out.status.success() {
        // Most likely "could not find Cargo.toml" — not a format break.
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    Some(json.get("version").and_then(|v| v.as_u64()) == Some(dx_compat::REQUIRED_METADATA_FORMAT))
}

pub fn cmd_compat() -> Result<ExitCode> {
    eprintln!("== dx compat (Cargo compatibility contract) ==");

    let cargo_line = first_line(&cargo(), &["--version"]).unwrap_or_default();
    let rustc_line = first_line("rustc", &["--version"]).unwrap_or_default();
    eprintln!("cargo: {cargo_line}");
    eprintln!("rustc: {rustc_line}");

    let mut fatal = false;

    // Version classification.
    match dx_compat::parse_version(&cargo_line) {
        Some(v) => {
            let (lo_maj, lo_min) = dx_compat::MSRV;
            let (hi_maj, hi_min) = dx_compat::MAX_TESTED;
            match classify(&v) {
                Support::Supported => eprintln!(
                    "  version: SUPPORTED (tested range {lo_maj}.{lo_min}..={hi_maj}.{hi_min})"
                ),
                Support::NewerUntested => eprintln!(
                    "  version: NEWER THAN TESTED (> {hi_maj}.{hi_min}) — allowed, not yet verified by CI"
                ),
                Support::TooOld => {
                    fatal = true;
                    eprintln!("  version: TOO OLD (< MSRV {lo_maj}.{lo_min}) — unsupported");
                }
            }
        }
        None => eprintln!("  version: could not parse `cargo --version`"),
    }

    // Interface probes.
    eprintln!("interfaces:");
    for iface in dx_compat::INTERFACES {
        let (status, broken) = probe(iface);
        let tag = if iface.required {
            "required"
        } else {
            "optional"
        };
        eprintln!("  [{status}] ({tag}) {} — {}", iface.probe, iface.purpose);
        if broken && iface.required {
            fatal = true;
        }
    }

    if fatal {
        eprintln!("\n\u{2717} compatibility: a REQUIRED interface is broken or Cargo is too old.");
        Ok(ExitCode::from(4))
    } else {
        eprintln!("\n\u{2713} compatibility: all required interfaces present.");
        Ok(ExitCode::SUCCESS)
    }
}

/// Returns (status label, is_broken). `is_broken` is only meaningful for
/// required interfaces; "n/a" (e.g. not in a project) is never broken.
fn probe(iface: &dx_compat::Interface) -> (&'static str, bool) {
    if iface.probe.starts_with("cargo metadata") {
        return match metadata_format_ok() {
            Some(true) => ("OK", false),
            Some(false) => ("FORMAT MISMATCH", true),
            None => ("n/a (not in a project)", false),
        };
    }
    if iface.probe.starts_with("rustc -vV") {
        let ok = first_line("rustc", &["-vV"]).is_some();
        return if ok { ("OK", false) } else { ("MISSING", true) };
    }
    // cargo subcommands: tree / generate-lockfile / update / vendor. Split on
    // whitespace only — hyphenated names like `generate-lockfile` are one token.
    let sub = iface
        .probe
        .strip_prefix("cargo ")
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or("");
    let ok = subcommand_available(sub);
    if ok {
        ("OK", false)
    } else {
        ("MISSING", true)
    }
}

/// One-line summary for `dx doctor`.
pub fn doctor_line() -> String {
    let cargo_line = first_line(&cargo(), &["--version"]).unwrap_or_default();
    match dx_compat::parse_version(&cargo_line) {
        Some(v) => {
            let (hi_maj, hi_min) = dx_compat::MAX_TESTED;
            let label = match classify(&v) {
                Support::Supported => "supported",
                Support::NewerUntested => "newer-than-tested",
                Support::TooOld => "TOO OLD",
            };
            format!(
                "cargo compatibility: {label} (tested up to {hi_maj}.{hi_min}; run `dx compat` to probe interfaces)"
            )
        }
        None => "cargo compatibility: unknown (could not parse cargo --version)".into(),
    }
}

/// Entry used by the Commands dispatch.
pub fn run() -> Result<ExitCode> {
    cmd_compat().context("probe Cargo compatibility")
}
