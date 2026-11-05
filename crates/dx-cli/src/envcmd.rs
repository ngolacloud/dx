//! `dx environment capture|verify` — record and verify the build environment.

use std::process::{Command, ExitCode};

use anyhow::{bail, Context, Result};
use camino::Utf8PathBuf;

use crate::security::now_unix;

const FILE: &str = "dx-environment.json";

fn locate_root() -> Result<Utf8PathBuf> {
    let cwd = std::env::current_dir().context("CWD")?;
    let mut dir = Utf8PathBuf::from_path_buf(cwd).map_err(|_| anyhow::anyhow!("non-UTF8 CWD"))?;
    loop {
        if dir.join("Cargo.toml").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            bail!("no Cargo.toml found from the current directory");
        }
    }
}

fn now_iso() -> String {
    Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| format!("{}s-epoch", now_unix()))
}

fn summary(env: &dx_env::Environment) {
    eprintln!("  rustc:        {}", env.rustc);
    eprintln!("  host target:  {}", env.host_target);
    eprintln!("  cargo:        {}", env.cargo);
    for t in &env.tools {
        eprintln!(
            "  tool {}: {}",
            t.name,
            t.version.as_deref().unwrap_or("absent")
        );
    }
    eprintln!(
        "  advisory-db:  {} (valid={})",
        env.security.advisory_db_snapshot, env.security.advisory_db_valid
    );
    eprintln!(
        "  lockfile:     {}",
        env.lockfile_sha256.as_deref().unwrap_or("(none)")
    );
}

pub fn cmd_capture() -> Result<ExitCode> {
    let root = locate_root()?;
    let env = dx_env::capture(&root, &now_iso(), now_unix());
    let path = root.join(FILE);
    std::fs::write(path.as_std_path(), dx_env::to_json(&env))
        .with_context(|| format!("write {path}"))?;
    eprintln!("== dx environment capture ==");
    summary(&env);
    eprintln!("\n\u{2713} Environment recorded in {path} (commit it for CI/teams).");
    Ok(ExitCode::SUCCESS)
}

pub fn cmd_verify() -> Result<ExitCode> {
    let root = locate_root()?;
    let path = root.join(FILE);
    let text = std::fs::read_to_string(path.as_std_path())
        .with_context(|| format!("read {path} (run `dx environment capture` first)"))?;
    let expected = dx_env::from_json(&text).with_context(|| format!("parse {path}"))?;
    let actual = dx_env::capture(&root, &now_iso(), now_unix());

    eprintln!("== dx environment verify ==");
    let diffs = dx_env::diff(&expected, &actual);
    let critical: Vec<&dx_env::Diff> = diffs.iter().filter(|d| d.critical).collect();
    let info: Vec<&dx_env::Diff> = diffs.iter().filter(|d| !d.critical).collect();

    if diffs.is_empty() {
        eprintln!("\u{2713} Environment matches the recorded one.");
        return Ok(ExitCode::SUCCESS);
    }
    for d in &critical {
        eprintln!(
            "  DRIFT  {}: expected `{}`, got `{}`",
            d.field, d.expected, d.actual
        );
    }
    for d in &info {
        eprintln!(
            "  info   {}: recorded `{}`, now `{}`",
            d.field, d.expected, d.actual
        );
    }
    if critical.is_empty() {
        eprintln!("\n\u{2713} No critical drift (only informational differences).");
        Ok(ExitCode::SUCCESS)
    } else {
        eprintln!(
            "\n\u{2717} {} critical drift(s) from the recorded environment.",
            critical.len()
        );
        Ok(ExitCode::from(4))
    }
}

pub fn run(action: &EnvAction) -> Result<ExitCode> {
    match action {
        EnvAction::Capture => cmd_capture(),
        EnvAction::Verify => cmd_verify(),
    }
}

#[derive(clap::Subcommand)]
pub enum EnvAction {
    /// Record the current environment to dx-environment.json.
    Capture,
    /// Verify the host matches the recorded environment.
    Verify,
}
