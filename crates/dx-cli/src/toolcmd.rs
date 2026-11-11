//! `dx tool install|run|list` — project-pinned developer tools with
//! provenance, verification and optional isolation. Tools are distinct from
//! libraries (`dx add`); `run` never falls back to a system binary.

use std::process::{Command, ExitCode};

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use dx_tool::{InstalledTool, ToolSource, ToolSpec};

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

fn tools_root(root: &Utf8Path) -> Utf8PathBuf {
    root.join(".dx/tools")
}
fn manifest_path(root: &Utf8Path) -> Utf8PathBuf {
    root.join("dx-tools.toml")
}
fn lock_path(root: &Utf8Path) -> Utf8PathBuf {
    tools_root(root).join("dx-tools.lock")
}
fn bin_path(root: &Utf8Path, name: &str) -> Utf8PathBuf {
    tools_root(root).join("bin").join(name)
}

fn read_specs(root: &Utf8Path) -> Result<Vec<ToolSpec>> {
    match std::fs::read_to_string(manifest_path(root).as_std_path()) {
        Ok(t) => {
            dx_tool::parse_manifest(&t).map_err(|e| anyhow::anyhow!("invalid dx-tools.toml: {e}"))
        }
        Err(_) => Ok(Vec::new()),
    }
}
fn read_lock(root: &Utf8Path) -> dx_tool::ToolLock {
    std::fs::read_to_string(lock_path(root).as_std_path())
        .ok()
        .and_then(|t| dx_tool::parse_lock(&t).ok())
        .unwrap_or(dx_tool::ToolLock {
            schema_version: 1,
            installed: vec![],
        })
}

fn source_label(s: &ToolSource) -> String {
    match s {
        ToolSource::Registry { registry: None } => "registry:crates.io".into(),
        ToolSource::Registry { registry: Some(r) } => format!("registry:{r}"),
        ToolSource::Path { path } => format!("path:{path}"),
    }
}

pub fn cmd_install(
    spec_arg: Option<&str>,
    path: Option<&str>,
    root_given: Option<&Utf8Path>,
) -> Result<ExitCode> {
    let root = root_given.map(|p| p.to_owned()).unwrap_or(locate_root()?);
    let mut specs = read_specs(&root)?;

    // Which tools to install: the named one, or all pinned in the manifest.
    let targets: Vec<ToolSpec> = if let Some(arg) = spec_arg {
        let (name, version) = match arg.split_once('@') {
            Some((n, v)) => (n.to_string(), Some(v.to_string())),
            None => (arg.to_string(), None),
        };
        let source = match path {
            Some(p) => ToolSource::Path {
                path: p.to_string(),
            },
            None => ToolSource::Registry { registry: None },
        };
        let spec = ToolSpec {
            name,
            version,
            source,
        };
        dx_tool::upsert(&mut specs, spec.clone());
        // Durável: lock + journal + rename atómico (ADR-0004), não `fs::write` nu (C-1).
        dx_manifest::apply_transaction(
            &root,
            &[dx_manifest::FileWrite {
                path: manifest_path(&root),
                new_contents: dx_tool::render_manifest(&specs),
            }],
        )
        .context("write dx-tools.toml")?;
        vec![spec]
    } else {
        if specs.is_empty() {
            bail!(
                "no tool given and dx-tools.toml has none; use `dx tool install <name>@<version>`"
            );
        }
        specs.clone()
    };

    let mut lock = read_lock(&root);
    std::fs::create_dir_all(tools_root(&root).as_std_path()).ok();

    for spec in &targets {
        eprintln!("== dx tool install {} ==", spec.name);
        let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        cmd.arg("install")
            .arg("--root")
            .arg(tools_root(&root).as_str())
            .arg("--locked");
        match &spec.source {
            ToolSource::Path { path } => {
                cmd.arg("--path").arg(path);
            }
            ToolSource::Registry { registry } => {
                cmd.arg(&spec.name);
                if let Some(v) = &spec.version {
                    cmd.args(["--version", v]);
                }
                if let Some(r) = registry {
                    cmd.args(["--registry", r]);
                }
            }
        }
        let status = cmd.status().context("cargo install")?;
        if !status.success() {
            bail!(
                "cargo install failed for `{}` (no silent fallback to a system binary)",
                spec.name
            );
        }
        let bin = bin_path(&root, &spec.name);
        if !bin.exists() {
            bail!(
                "installed but binary `{}` not found at {bin} (the crate's bin name must match `{}`)",
                spec.name, spec.name
            );
        }
        let sha = dx_integrity::sha256_file(&bin).context("hash installed binary")?;
        let version = spec.version.clone().unwrap_or_else(|| "unspecified".into());
        lock.upsert(InstalledTool {
            name: spec.name.clone(),
            version: version.clone(),
            source: source_label(&spec.source),
            sha256: sha.clone(),
            binary: bin.to_string(),
        });
        eprintln!("  installed: {} {}", spec.name, version);
        eprintln!("  source:    {}", source_label(&spec.source));
        eprintln!("  binary:    {bin}");
        eprintln!("  sha256:    {sha}");
    }

    // Durável: lock + journal + rename atómico (ADR-0004), não `fs::write` nu (C-1).
    dx_manifest::apply_transaction(
        &root,
        &[dx_manifest::FileWrite {
            path: lock_path(&root),
            new_contents: dx_tool::render_lock(&lock),
        }],
    )
    .context("write dx-tools.lock")?;
    eprintln!(
        "\n\u{2713} Pinned in dx-tools.toml; provenance in {}.",
        lock_path(&root)
    );
    Ok(ExitCode::SUCCESS)
}

pub fn cmd_run(name: &str, args: &[String], isolated: bool) -> Result<ExitCode> {
    let root = locate_root()?;
    let lock = read_lock(&root);
    let entry = match lock.find(name) {
        Some(e) => e.clone(),
        None => bail!(
            "tool `{name}` is not installed for this project; run `dx tool install {name}@<version>` \
             (no silent fallback to a system binary)"
        ),
    };
    let bin = bin_path(&root, name);
    if !bin.exists() {
        bail!(
            "tool `{name}` recorded but binary missing at {bin}; reinstall with `dx tool install`"
        );
    }
    // Provenance / tamper check.
    let sha = dx_integrity::sha256_file(&bin).context("hash tool binary")?;
    if sha != entry.sha256 {
        bail!(
            "tool `{name}` binary does NOT match recorded provenance (expected {}, got {sha}); refusing to run",
            entry.sha256
        );
    }

    if isolated {
        let caps = dx_sandbox::Capabilities::detect();
        let spec = dx_sandbox::SandboxSpec::default();
        let outcome =
            dx_sandbox::run(&spec, bin.as_str(), args, &caps).context("run tool isolated")?;
        eprintln!(
            "(isolated: network={} env-scrubbed={})",
            outcome.enforced.network_isolated, outcome.enforced.env_scrubbed
        );
        return Ok(ExitCode::from(outcome.code.unwrap_or(1) as u8));
    }

    let status = Command::new(bin.as_str())
        .args(args)
        .status()
        .context("run tool")?;
    Ok(ExitCode::from(status.code().unwrap_or(130) as u8))
}

pub fn cmd_list() -> Result<ExitCode> {
    let root = locate_root()?;
    let specs = read_specs(&root)?;
    let lock = read_lock(&root);
    eprintln!("== dx tool list ==");
    if specs.is_empty() {
        eprintln!("(no tools pinned in dx-tools.toml)");
        return Ok(ExitCode::SUCCESS);
    }
    for s in &specs {
        let installed = lock.find(&s.name);
        let status = match installed {
            Some(i) => format!(
                "installed {} [{}]",
                i.version,
                &i.sha256[..i.sha256.len().min(12)]
            ),
            None => "NOT installed".into(),
        };
        eprintln!(
            "  {} ({}, pinned {}) — {status}",
            s.name,
            source_label(&s.source),
            s.version.as_deref().unwrap_or("*")
        );
    }
    Ok(ExitCode::SUCCESS)
}

pub fn run(action: ToolAction) -> Result<ExitCode> {
    match action {
        ToolAction::Install { spec, path } => cmd_install(spec.as_deref(), path.as_deref(), None),
        ToolAction::Run {
            name,
            isolated,
            args,
        } => cmd_run(&name, &args, isolated),
        ToolAction::List => cmd_list(),
    }
}

#[derive(clap::Subcommand)]
pub enum ToolAction {
    /// Install/pin a tool (name@version) into the project (.dx/tools).
    Install {
        /// `name` or `name@version`; omit to install all pinned tools.
        spec: Option<String>,
        /// Install from a local path crate (controlled/offline).
        #[arg(long)]
        path: Option<String>,
    },
    /// Run a project-pinned tool (verifies provenance; no system fallback).
    Run {
        name: String,
        /// Run under real isolation (network off, environment scrubbed).
        #[arg(long)]
        isolated: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// List pinned tools and their install/provenance status.
    List,
}
