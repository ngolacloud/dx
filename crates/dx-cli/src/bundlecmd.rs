//! `dx bundle export|verify` — offline, verifiable build bundle.
//!
//! Bundles the environment record, SBOM, Cargo.toml/Cargo.lock and the vendored
//! dependency sources into one directory, with a `bundle.json` manifest that
//! records a sha256 per top-level file and an aggregate hash of `vendor/`.
//! `verify` recomputes the hashes and reports tampering. `reproduce` re-vendors
//! from the bundle's Cargo.lock and proves the dependency sources are
//! byte-for-byte reproducible (ADR-0027). Honest scope: the *vendored sources*
//! reproduce for a given lockfile; the timestamp and host environment record do
//! not, and are excluded from that claim.

use std::process::{Command, ExitCode};

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde_json::json;
use tempfile::TempDir;

use crate::security::now_unix;

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

/// Recursively hash a directory: returns (file_count, aggregate_sha256_hex).
/// The aggregate is the sha256 of the sorted "relpath:filehash" lines, so it is
/// deterministic and detects any content/addition/removal.
fn hash_tree(dir: &Utf8Path) -> (usize, String) {
    use sha2::{Digest, Sha256};
    let mut entries: Vec<(String, String)> = Vec::new();
    fn walk(base: &Utf8Path, cur: &Utf8Path, out: &mut Vec<(String, String)>) {
        let rd = match std::fs::read_dir(cur.as_std_path()) {
            Ok(r) => r,
            Err(_) => return,
        };
        for e in rd.flatten() {
            let p = e.path();
            let up = match Utf8PathBuf::from_path_buf(p) {
                Ok(u) => u,
                Err(_) => continue,
            };
            if up.is_dir() {
                walk(base, &up, out);
            } else if let Ok(h) = dx_integrity::sha256_file(&up) {
                let rel = up
                    .as_str()
                    .strip_prefix(base.as_str())
                    .unwrap_or(up.as_str())
                    .trim_start_matches('/')
                    .to_string();
                out.push((rel, h));
            }
        }
    }
    if dir.exists() {
        walk(dir, dir, &mut entries);
    }
    entries.sort();
    let mut h = Sha256::new();
    for (rel, fh) in &entries {
        h.update(rel.as_bytes());
        h.update(b":");
        h.update(fh.as_bytes());
        h.update(b"\n");
    }
    let agg = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    (entries.len(), agg)
}

fn copy_into(src: &Utf8Path, dst: &Utf8Path) -> Result<()> {
    std::fs::copy(src.as_std_path(), dst.as_std_path())
        .with_context(|| format!("copy {src} -> {dst}"))?;
    Ok(())
}

pub fn cmd_export(out: Option<&Utf8Path>) -> Result<ExitCode> {
    let root = locate_root()?;
    let lockfile = root.join("Cargo.lock");
    if !lockfile.exists() {
        bail!("no Cargo.lock (run `dx add`/resolve first) — a bundle needs a locked graph");
    }
    let out = out
        .map(|p| p.to_owned())
        .unwrap_or_else(|| root.join("dx-bundle"));
    std::fs::create_dir_all(out.as_std_path()).with_context(|| format!("create {out}"))?;
    let generated = now_iso();

    eprintln!("== dx bundle export == ({out})");

    // 1) Cargo.toml + Cargo.lock
    copy_into(&root.join("Cargo.toml"), &out.join("Cargo.toml"))?;
    copy_into(&lockfile, &out.join("Cargo.lock"))?;

    // 2) Environment record
    let env = dx_env::capture(&root, &generated, now_unix());
    std::fs::write(
        out.join("environment.json").as_std_path(),
        dx_env::to_json(&env),
    )
    .context("write environment.json")?;

    // 3) SBOM
    let bom = dx_release::sbom::cyclonedx(&lockfile, &generated).context("generate SBOM")?;
    std::fs::write(
        out.join("sbom.cdx.json").as_std_path(),
        serde_json::to_vec_pretty(&bom).unwrap_or_default(),
    )
    .context("write sbom.cdx.json")?;

    // 4) Vendor the dependency sources (needs network; consumed offline later).
    eprintln!("Vendoring dependency sources...");
    let vendor = out.join("vendor");
    let v = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .arg("vendor")
        .arg(vendor.as_str())
        .current_dir(root.as_std_path())
        .output()
        .context("cargo vendor")?;
    if !v.status.success() {
        bail!(
            "cargo vendor failed:\n{}",
            String::from_utf8_lossy(&v.stderr)
        );
    }
    // The stdout is the [source.*] config to use offline.
    std::fs::write(out.join("cargo-config.toml").as_std_path(), &v.stdout)
        .context("write cargo-config.toml")?;

    // 5) bundle.json with per-file sha256 + vendor aggregate.
    let top = [
        "Cargo.toml",
        "Cargo.lock",
        "environment.json",
        "sbom.cdx.json",
        "cargo-config.toml",
    ];
    let mut files = Vec::new();
    for name in top {
        let p = out.join(name);
        if let Ok(h) = dx_integrity::sha256_file(&p) {
            let bytes = std::fs::metadata(p.as_std_path())
                .map(|m| m.len())
                .unwrap_or(0);
            files.push(json!({ "path": name, "sha256": h, "bytes": bytes }));
        }
    }
    let (vcount, vagg) = hash_tree(&vendor);
    let manifest = json!({
        "schema_version": 1,
        "generated": generated,
        "files": files,
        "vendor": { "files": vcount, "aggregate_sha256": vagg },
    });
    std::fs::write(
        out.join("bundle.json").as_std_path(),
        serde_json::to_vec_pretty(&manifest).unwrap_or_default(),
    )
    .context("write bundle.json")?;

    eprintln!("  Cargo.toml, Cargo.lock, environment.json, sbom.cdx.json, cargo-config.toml");
    eprintln!(
        "  vendor/: {vcount} file(s), aggregate {}",
        &vagg[..vagg.len().min(12)]
    );
    eprintln!("\n\u{2713} Verifiable bundle written to {out} (offline-consumable; verify with `dx bundle verify {out}`).");
    Ok(ExitCode::SUCCESS)
}

pub fn cmd_verify(dir: &Utf8Path) -> Result<ExitCode> {
    let manifest_path = dir.join("bundle.json");
    let text = std::fs::read_to_string(manifest_path.as_std_path())
        .with_context(|| format!("read {manifest_path} (is this a dx bundle?)"))?;
    let m: serde_json::Value = serde_json::from_str(&text).context("parse bundle.json")?;

    eprintln!("== dx bundle verify == ({dir})");
    let mut bad = 0usize;

    if let Some(files) = m.get("files").and_then(|f| f.as_array()) {
        for f in files {
            let path = f.get("path").and_then(|x| x.as_str()).unwrap_or("");
            let want = f.get("sha256").and_then(|x| x.as_str()).unwrap_or("");
            let p = dir.join(path);
            match dx_integrity::sha256_file(&p) {
                Ok(h) if h == want => {}
                Ok(_) => {
                    eprintln!("  MISMATCH {path}");
                    bad += 1;
                }
                Err(_) => {
                    eprintln!("  MISSING  {path}");
                    bad += 1;
                }
            }
        }
    }
    if let Some(v) = m.get("vendor") {
        let want = v
            .get("aggregate_sha256")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let (_, got) = hash_tree(&dir.join("vendor"));
        if got != want {
            eprintln!("  MISMATCH vendor/ (aggregate hash)");
            bad += 1;
        }
    }

    if bad == 0 {
        eprintln!("\u{2713} Bundle integrity OK (all files and vendor/ match).");
        Ok(ExitCode::SUCCESS)
    } else {
        eprintln!("\n\u{2717} {bad} bundle integrity failure(s).");
        Ok(ExitCode::from(4))
    }
}

/// `dx bundle reproduce <dir>` — prove the vendored dependency sources are
/// byte-for-byte reproducible (ADR-0027).
///
/// Re-vendors from the bundle's own `Cargo.lock` into a scratch dir and compares
/// the `vendor/` aggregate hash against the one recorded in `bundle.json`. The
/// timestamp and `environment.json` are host/time-specific and deliberately
/// excluded — only the dependency sources (the code you would build) must be
/// deterministic for a given lockfile.
pub fn cmd_reproduce(dir: &Utf8Path) -> Result<ExitCode> {
    let manifest_path = dir.join("bundle.json");
    let text = std::fs::read_to_string(manifest_path.as_std_path())
        .with_context(|| format!("read {manifest_path} (is this a dx bundle?)"))?;
    let m: serde_json::Value = serde_json::from_str(&text).context("parse bundle.json")?;
    let want = m
        .get("vendor")
        .and_then(|v| v.get("aggregate_sha256"))
        .and_then(|x| x.as_str())
        .ok_or_else(|| anyhow::anyhow!("bundle.json has no vendor aggregate to reproduce"))?;

    let cargo_toml = dir.join("Cargo.toml");
    let cargo_lock = dir.join("Cargo.lock");
    if !cargo_toml.exists() || !cargo_lock.exists() {
        bail!("bundle is missing Cargo.toml/Cargo.lock — cannot re-vendor");
    }

    eprintln!("== dx bundle reproduce == ({dir})");
    let guard = TempDir::new().context("create scratch dir")?;
    let scratch = Utf8Path::from_path(guard.path())
        .ok_or_else(|| anyhow::anyhow!("non-UTF8 scratch path"))?
        .to_owned();
    copy_into(&cargo_toml, &scratch.join("Cargo.toml"))?;
    copy_into(&cargo_lock, &scratch.join("Cargo.lock"))?;
    // A minimal target so cargo accepts the manifest; the vendored dependency
    // sources do not depend on our own source. `main.rs` satisfies both bin and
    // lib manifests.
    std::fs::create_dir_all(scratch.join("src").as_std_path()).context("create scratch src/")?;
    std::fs::write(scratch.join("src/main.rs").as_std_path(), "fn main() {}\n")
        .context("write scratch main.rs")?;

    eprintln!("Re-vendoring from the bundle's Cargo.lock...");
    let vendor = scratch.join("vendor");
    let v = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .arg("vendor")
        .arg(vendor.as_str())
        .current_dir(scratch.as_std_path())
        .output()
        .context("cargo vendor")?;
    if !v.status.success() {
        bail!(
            "cargo vendor failed:\n{}",
            String::from_utf8_lossy(&v.stderr)
        );
    }

    let (count, got) = hash_tree(&vendor);
    eprintln!("  re-vendored {count} file(s)");
    eprintln!("  recorded aggregate:    {want}");
    eprintln!("  reproduced aggregate:  {got}");
    if got == want {
        eprintln!("\n\u{2713} Reproducible: the vendored dependency sources match the bundle byte-for-byte.");
        Ok(ExitCode::SUCCESS)
    } else {
        eprintln!(
            "\n\u{2717} NOT reproducible: the re-vendored sources differ from the bundle's recorded hash."
        );
        Ok(ExitCode::from(4))
    }
}

pub fn run(action: BundleAction) -> Result<ExitCode> {
    match action {
        BundleAction::Export { out } => cmd_export(out.as_deref()),
        BundleAction::Verify { dir } => cmd_verify(&dir),
        BundleAction::Reproduce { dir } => cmd_reproduce(&dir),
    }
}

#[derive(clap::Subcommand)]
pub enum BundleAction {
    /// Export a verifiable offline bundle (env + SBOM + vendored sources).
    Export {
        /// Output directory (default: dx-bundle).
        #[arg(long)]
        out: Option<Utf8PathBuf>,
    },
    /// Verify a bundle's integrity (file hashes + vendor aggregate).
    Verify {
        /// Bundle directory.
        dir: Utf8PathBuf,
    },
    /// Re-vendor from the bundle's Cargo.lock and prove the sources reproduce.
    Reproduce {
        /// Bundle directory.
        dir: Utf8PathBuf,
    },
}
