//! `dx toolchain list|install|pin|use` — project toolchain pinning.
//!
//! DX **composes** rustup (ADR-0022): the pin it writes is the native
//! `rust-toolchain.toml`, so cargo/rustup honour it without DX. DX adds a
//! provenance record (`dx-toolchain.lock`) of what the channel resolved to, and
//! `dx environment verify` points here when it sees toolchain drift. No
//! fallback: if `rustup` is absent, DX refuses honestly rather than guessing.

use std::process::{Command, ExitCode, Stdio};

use anyhow::{bail, Context, Result};
use camino::Utf8PathBuf;
use dx_manifest::{apply_transaction, FileWrite};
use dx_toolchain::{Pin, Provenance, LOCK_FILE, TOOLCHAIN_FILE};

#[derive(clap::Subcommand)]
pub enum ToolchainAction {
    /// List installed toolchains and the project pin/provenance.
    List,
    /// Install a toolchain via rustup and record its provenance.
    Install {
        /// Channel/version (e.g. `1.82`, `stable`, `nightly-2026-01-01`).
        version: String,
    },
    /// Pin the project to a toolchain (writes rust-toolchain.toml).
    Pin {
        /// Channel/version to pin.
        version: String,
    },
    /// Install (if missing) + pin + record provenance.
    Use {
        /// Channel/version to install and pin.
        version: String,
    },
}

pub fn run(action: &ToolchainAction) -> Result<ExitCode> {
    match action {
        ToolchainAction::List => cmd_list(),
        ToolchainAction::Install { version } => cmd_install(version),
        ToolchainAction::Pin { version } => cmd_pin(version),
        ToolchainAction::Use { version } => cmd_use(version),
    }
}

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

fn require_rustup() -> Result<()> {
    let ok = Command::new("rustup")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        bail!(
            "rustup not found. DX composes rustup to manage toolchains and does not replace it — install rustup (https://rustup.rs) and retry."
        )
    }
}

/// First tokens of `rustup toolchain list` (the toolchain names).
fn installed_toolchains() -> Vec<String> {
    Command::new("rustup")
        .args(["toolchain", "list"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| l.split_whitespace().next().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether the **exact** channel identity is already installed. rustup treats
/// `1.96` (latest 1.96.x) and `1.96.0` as distinct toolchains, so this matches
/// only `channel` or `channel-<host-triple>` — never `1.96.0` for `1.96`. This
/// guarantees a `dx toolchain pin`/provenance probe never triggers a download:
/// if the exact channel isn't present, we print an "install first" note instead.
fn is_installed(channel: &str) -> bool {
    installed_toolchains()
        .iter()
        .any(|t| t == channel || t.starts_with(&format!("{channel}-")))
}

/// Provenance of a pinned channel: runs `rustc -vV` **in the project dir** so a
/// shorthand pin (e.g. `1.96`) resolves through `rust-toolchain.toml` to the
/// concrete toolchain (`1.96.0`). Only call when `is_installed(channel)` — the
/// proxy would otherwise auto-install the missing toolchain.
fn resolve_in_dir(root: &Utf8PathBuf, channel: &str) -> Option<Provenance> {
    let out = Command::new("rustc")
        .arg("-vV")
        .current_dir(root.as_std_path())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let vv = String::from_utf8_lossy(&out.stdout);
    Some(dx_toolchain::parse_rustc_vv(channel, &vv))
}

/// Provenance of an exactly-named, already-installed toolchain.
fn resolve_exact(channel: &str) -> Option<Provenance> {
    let out = Command::new("rustup")
        .args(["run", channel, "rustc", "-vV"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let vv = String::from_utf8_lossy(&out.stdout);
    Some(dx_toolchain::parse_rustc_vv(channel, &vv))
}

fn write_provenance(root: &Utf8PathBuf, prov: &Provenance) -> Result<()> {
    let path = root.join(LOCK_FILE);
    apply_transaction(
        root,
        &[FileWrite {
            path: path.clone(),
            new_contents: dx_toolchain::provenance_to_json(prov),
        }],
    )
    .with_context(|| format!("write {path}"))?;
    eprintln!(
        "\u{2713} provenance recorded in {path}: {} (host {})",
        prov.rustc_version, prov.host
    );
    Ok(())
}

fn cmd_list() -> Result<ExitCode> {
    require_rustup()?;
    eprintln!("== installed toolchains (rustup) ==");
    Command::new("rustup")
        .args(["toolchain", "list"])
        .status()
        .context("invoke rustup toolchain list")?;

    let root = locate_root()?;
    match dx_toolchain::read_pin(&root) {
        Some(pin) => {
            eprintln!("\n== project pin ({TOOLCHAIN_FILE}) ==");
            eprintln!("  channel:    {}", pin.channel);
            if !pin.components.is_empty() {
                eprintln!("  components: {}", pin.components.join(", "));
            }
            if !pin.targets.is_empty() {
                eprintln!("  targets:    {}", pin.targets.join(", "));
            }
        }
        None => eprintln!("\n== project pin == (none — run `dx toolchain pin <version>`)"),
    }
    if let Some(prov) = dx_toolchain::read_provenance(&root) {
        eprintln!("\n== recorded provenance ({LOCK_FILE}) ==");
        eprintln!("  {} resolves to {}", prov.channel, prov.rustc_version);
        if let Some(h) = &prov.commit_hash {
            eprintln!(
                "  commit: {h} ({})",
                prov.commit_date.as_deref().unwrap_or("?")
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn install(version: &str) -> Result<()> {
    eprintln!("== dx toolchain install {version} ==");
    let status = Command::new("rustup")
        .args(["toolchain", "install", version])
        .status()
        .context("invoke rustup toolchain install")?;
    if !status.success() {
        bail!("rustup failed to install toolchain `{version}`");
    }
    Ok(())
}

fn cmd_install(version: &str) -> Result<ExitCode> {
    require_rustup()?;
    install(version)?;
    let root = locate_root()?;
    match resolve_exact(version) {
        Some(prov) => write_provenance(&root, &prov)?,
        None => eprintln!(
            "\u{26a0} installed, but could not resolve provenance for `{version}` (is the channel name exact?)."
        ),
    }
    Ok(ExitCode::SUCCESS)
}

fn pin(root: &Utf8PathBuf, version: &str) -> Result<()> {
    let path = root.join(TOOLCHAIN_FILE);
    let existing = std::fs::read_to_string(path.as_std_path()).ok();
    let mut p = Pin::new(version);
    // Preserve components/targets already pinned, only swap the channel.
    if let Some(prev) = existing.as_deref().and_then(dx_toolchain::parse_pin) {
        p.components = prev.components;
        p.targets = prev.targets;
    }
    let rendered = dx_toolchain::upsert_pin(existing.as_deref(), &p);
    // Durável: lock + journal + rename atómico (ADR-0004), não `fs::write` nu — um
    // crash deixaria o `rust-toolchain.toml` truncado; dois processos, lost-update (C-1).
    apply_transaction(
        root,
        &[FileWrite {
            path: path.clone(),
            new_contents: rendered,
        }],
    )
    .with_context(|| format!("write {path}"))?;
    eprintln!(
        "\u{2713} pinned `{version}` in {path} (standard rust-toolchain.toml — works without DX)."
    );
    Ok(())
}

fn cmd_pin(version: &str) -> Result<ExitCode> {
    require_rustup()?;
    let root = locate_root()?;
    pin(&root, version)?;
    // Record provenance ONLY if the channel is already installed — never let a
    // pin trigger a toolchain download.
    if is_installed(version) {
        match resolve_in_dir(&root, version) {
            Some(prov) => write_provenance(&root, &prov)?,
            None => eprintln!("note: could not resolve provenance for `{version}`."),
        }
    } else {
        eprintln!(
            "note: `{version}` is not installed yet — run `dx toolchain install {version}` (or `dx toolchain use {version}`) to record provenance."
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_use(version: &str) -> Result<ExitCode> {
    require_rustup()?;
    install(version)?;
    let root = locate_root()?;
    pin(&root, version)?;
    // Installed just above; resolve through the pin (shorthand → concrete).
    match resolve_in_dir(&root, version).or_else(|| resolve_exact(version)) {
        Some(prov) => write_provenance(&root, &prov)?,
        None => eprintln!("\u{26a0} pinned, but could not resolve provenance for `{version}`."),
    }
    Ok(ExitCode::SUCCESS)
}
