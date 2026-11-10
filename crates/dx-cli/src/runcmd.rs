//! `dx run <script.rs>` — run a single-file Rust script with inline deps.
//!
//! The DX differentiator over `cargo -Zscript`/`uvx`: the script's dependency
//! graph passes the **same security gate** as `dx add` BEFORE the script runs.
//! We build an ephemeral package in a temp dir, resolve its lockfile without
//! compiling (`cargo generate-lockfile`), run the gate, and only `cargo run` it
//! if the verdict is `Approved`. `Blocked`/`Pending` refuse to execute.

use std::process::{Command, ExitCode};

use anyhow::{bail, Context, Result};
use camino::Utf8Path;
use dx_security::{run_gate, Decision, Policy};
use tempfile::TempDir;

use crate::security::{now_unix, report_verdict};

/// If the first arg is an existing `.rs` file, return (script, script_args).
pub fn detect_script(args: &[String]) -> Option<(&str, &[String])> {
    let first = args.first()?;
    if first.ends_with(".rs") && std::path::Path::new(first).is_file() {
        Some((first.as_str(), &args[1..]))
    } else {
        None
    }
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

pub fn run_script(path: &str, script_args: &[String], offline: bool) -> Result<ExitCode> {
    let src = std::fs::read_to_string(path).with_context(|| format!("read script {path}"))?;
    let script = dx_script::parse(&src);

    let stem = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("script");
    let pkg_name = dx_script::sanitize_pkg_name(stem);

    let manifest = dx_script::ephemeral_manifest(&script.manifest, &pkg_name)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Ephemeral package in a temp dir (guard cleans up on drop).
    let guard = TempDir::new().context("create ephemeral script package")?;
    let root = Utf8Path::from_path(guard.path())
        .ok_or_else(|| anyhow::anyhow!("non-UTF8 temp path"))?
        .to_owned();
    std::fs::create_dir_all(root.join("src")).context("create ephemeral src/")?;
    std::fs::write(root.join("Cargo.toml"), &manifest).context("write ephemeral Cargo.toml")?;
    std::fs::write(root.join("src/main.rs"), &script.code).context("write ephemeral main.rs")?;

    // Resolve the lockfile WITHOUT compiling or running build.rs.
    let mut gen = Command::new(cargo());
    gen.args(["generate-lockfile", "--manifest-path"])
        .arg(root.join("Cargo.toml").as_str());
    if offline {
        gen.arg("--offline");
    }
    let out = gen.output().context("invoke cargo generate-lockfile")?;
    if !out.status.success() {
        bail!(
            "could not resolve script dependencies:\n  {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    // Same gate as `dx add`: audit the exact graph before executing.
    let policy = Policy::from_env();
    let outcome = run_gate(&root, &root, &policy, now_unix())
        .context("run the security gate (is cargo-audit installed?)")?;
    report_verdict(
        &outcome.verdict,
        &root,
        outcome.from_cache,
        outcome.deny_covered,
    );

    match outcome.verdict.decision {
        Decision::Approved => {
            // Skip the leading `--` if the user wrote `dx run s.rs -- args`.
            let forwarded: &[String] = match script_args.first() {
                Some(s) if s == "--" => &script_args[1..],
                _ => script_args,
            };
            let mut cmd = Command::new(cargo());
            cmd.args(["run", "--manifest-path"])
                .arg(root.join("Cargo.toml").as_str());
            if offline {
                cmd.arg("--offline");
            }
            if !forwarded.is_empty() {
                cmd.arg("--");
                cmd.args(forwarded);
            }
            // status() (not exec) so the TempDir guard cleans up afterwards.
            let status = cmd.status().context("run the script")?;
            Ok(ExitCode::from(status.code().unwrap_or(130) as u8))
        }
        Decision::Blocked => {
            eprintln!(
                "\u{2717} script did NOT run: dependency graph blocked by the security gate."
            );
            Ok(ExitCode::from(Decision::Blocked.exit_code()))
        }
        Decision::Pending => {
            eprintln!(
                "\u{23f8} script did NOT run: advisory base invalid/expired. Renew it (`dx doctor`) and retry — resolution may have happened, execution is NOT authorized."
            );
            Ok(ExitCode::from(Decision::Pending.exit_code()))
        }
    }
}
