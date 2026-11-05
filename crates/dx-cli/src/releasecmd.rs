//! `dx release prepare` — valida em ambiente limpo e produz artefactos para
//! revisão (binário+checksum, SBOM, proveniência, ordem de publicação).
//! **Prepara ≠ publica**: não cria tags, não muda versões, não publica.
//! Idempotente: reescreve `target/dx-release/`.

use std::process::{Command, ExitCode};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use dx_manifest::Manifest;
use dx_release::{
    artifact_of, members_from_metadata, publish_order, sbom, write_review_bundle, Provenance,
};
use dx_security::{run_gate, Decision, Policy};

use crate::security::now_unix;

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

fn locate() -> Result<(Utf8PathBuf, Option<String>)> {
    let cwd = std::env::current_dir().context("CWD")?;
    let mut dir = Utf8PathBuf::from_path_buf(cwd).map_err(|_| anyhow::anyhow!("CWD não-UTF8"))?;
    loop {
        if dir.join("Cargo.toml").exists() {
            let name = Manifest::load(dir.join("Cargo.toml"))
                .ok()
                .and_then(|m| m.package_name().map(str::to_string));
            return Ok((dir.clone(), name));
        }
        if !dir.pop() {
            anyhow::bail!("não encontrei um Cargo.toml a partir do directório actual");
        }
    }
}

/// Corre um gate capturando a saída; devolve (ok, cauda-da-saída-se-falhou).
fn gate(label: &str, root: &Utf8Path, program: &str, args: &[&str]) -> (bool, String) {
    eprint!("  {label:<22} ... ");
    let out = Command::new(program)
        .args(args)
        .current_dir(root.as_std_path())
        .output();
    match out {
        Ok(o) if o.status.success() => {
            eprintln!("OK");
            (true, String::new())
        }
        Ok(o) => {
            eprintln!("FALHOU");
            let mut s = String::from_utf8_lossy(&o.stderr).into_owned();
            if s.len() > 800 {
                s = format!("...{}", &s[s.len() - 800..]);
            }
            (false, s)
        }
        Err(e) => {
            eprintln!("ERRO ({e})");
            (false, e.to_string())
        }
    }
}

fn capture(program: &str, args: &[&str], cwd: Option<&Utf8Path>) -> Option<String> {
    let mut c = Command::new(program);
    c.args(args);
    if let Some(d) = cwd {
        c.current_dir(d.as_std_path());
    }
    c.output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn host_target() -> String {
    capture("rustc", &["-vV"], None)
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("host: ").map(str::to_string))
        })
        .unwrap_or_else(|| "desconhecido".into())
}

pub fn cmd_release_prepare() -> Result<ExitCode> {
    let (root, pkg) = locate()?;
    eprintln!("== dx release prepare == ({root})");
    eprintln!("Prepara ≠ publica: NÃO cria tags, NÃO muda versões, NÃO publica.\n");

    // --- Gates em ambiente limpo ---
    eprintln!("Validações:");
    let mut failures: Vec<(String, String)> = Vec::new();
    let mut run = |label: &str, prog: &str, args: &[&str]| {
        let (ok, tail) = gate(label, &root, prog, args);
        if !ok {
            failures.push((label.to_string(), tail));
        }
    };
    run("fmt --check", &cargo(), &["fmt", "--", "--check"]);
    run(
        "lint (clippy -D)",
        &cargo(),
        &["clippy", "--all-targets", "--", "-D", "warnings"],
    );
    run("testes", &cargo(), &["test", "--quiet"]);
    run("build limpo (release)", &cargo(), &["build", "--release"]);

    // Gate de segurança do DX (inclui "refresh obrigatório antes de release").
    eprint!("  {:<22} ... ", "segurança (gate DX)");
    let policy = Policy::from_env();
    match run_gate(&root, &root, &policy, now_unix()) {
        Ok(o) if o.verdict.decision == Decision::Approved => eprintln!("OK"),
        Ok(o) => {
            eprintln!("{}", o.verdict.decision.as_str().to_uppercase());
            failures.push((
                "segurança".into(),
                format!(
                    "decisão {} — {}",
                    o.verdict.decision.as_str(),
                    o.verdict.db_status.detail
                ),
            ));
        }
        Err(e) => {
            eprintln!("ERRO");
            failures.push(("segurança".into(), e.to_string()));
        }
    }

    // Compatibilidade de API/versões: só se a ferramenta existir.
    eprint!("  {:<22} ... ", "semver (API)");
    if which("cargo-semver-checks") {
        let (ok, tail) = gate("", &root, &cargo(), &["semver-checks", "check-release"]);
        let _ = ok;
        if !tail.is_empty() {
            failures.push(("semver".into(), tail));
        }
    } else {
        eprintln!("SEM COBERTURA (instalar cargo-semver-checks)");
    }

    if !failures.is_empty() {
        eprintln!(
            "\n✗ NÃO PRONTO: {} validação(ões) falharam. Artefactos não produzidos.",
            failures.len()
        );
        for (name, tail) in &failures {
            eprintln!("  - {name}");
            if !tail.trim().is_empty() {
                eprintln!("      {}", tail.lines().last().unwrap_or("").trim());
            }
        }
        return Ok(ExitCode::from(4));
    }

    // --- Artefactos para revisão ---
    eprintln!("\nArtefactos:");
    let generated = capture("date", &["-u", "+%Y-%m-%dT%H:%M:%SZ"], None)
        .unwrap_or_else(|| format!("{}s-epoch", now_unix()));

    // SBOM a partir do Cargo.lock.
    let lockfile = root.join("Cargo.lock");
    let bom = sbom::cyclonedx(&lockfile, &generated).context("gerar SBOM")?;

    // Ordem topológica de publicação (workspace).
    let meta = capture(
        &cargo(),
        &["metadata", "--no-deps", "--format-version", "1"],
        Some(&root),
    )
    .context("cargo metadata")?;
    let members = members_from_metadata(&meta).context("parse metadata")?;
    let order = publish_order(&members).map_err(|e| anyhow::anyhow!("{e}"))?;

    // Binário(s) + checksums: alvos `bin` do metadata (ou o package do root).
    let mut artifacts = Vec::new();
    let mut bin_names = bin_targets(&meta);
    if bin_names.is_empty() {
        if let Some(name) = &pkg {
            bin_names.push(name.clone());
        }
    }
    for name in &bin_names {
        let bin = root.join("target/release").join(name);
        if bin.exists() {
            artifacts.push(artifact_of(&bin).context("checksum do binário")?);
        }
    }

    let provenance = Provenance {
        tool: "dx".into(),
        generated: generated.clone(),
        rustc: capture("rustc", &["--version"], None).unwrap_or_default(),
        host_target: host_target(),
        git_commit: capture("git", &["rev-parse", "HEAD"], Some(&root)),
        artifacts: artifacts.clone(),
    };

    let dir = write_review_bundle(&root, &bom, &provenance, &order).context("escrever bundle")?;

    eprintln!("  SBOM (CycloneDX):   {}", dir.join("sbom.cdx.json"));
    eprintln!("  proveniência:       {}", dir.join("provenance.json"));
    eprintln!("  ordem de publicação:{}", dir.join("publish-order.txt"));
    eprintln!("  binários+checksum:  {} artefacto(s)", artifacts.len());
    eprintln!("\nOrdem de publicação ({} membros):", order.len());
    eprintln!("  {}", order.join(" → "));
    eprintln!("\n✓ PRONTO PARA REVISÃO. Publicar requer instrução específica (não feito aqui).");
    Ok(ExitCode::SUCCESS)
}

/// Extrai os nomes dos alvos `bin` da saída de `cargo metadata`.
fn bin_targets(meta: &str) -> Vec<String> {
    let v: serde_json::Value = match serde_json::from_str(meta) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut names = Vec::new();
    if let Some(pkgs) = v.get("packages").and_then(|p| p.as_array()) {
        for p in pkgs {
            if let Some(targets) = p.get("targets").and_then(|t| t.as_array()) {
                for t in targets {
                    let is_bin = t
                        .get("kind")
                        .and_then(|k| k.as_array())
                        .map(|a| a.iter().any(|x| x.as_str() == Some("bin")))
                        .unwrap_or(false);
                    if is_bin {
                        if let Some(n) = t.get("name").and_then(|n| n.as_str()) {
                            names.push(n.to_string());
                        }
                    }
                }
            }
        }
    }
    names
}

fn which(bin: &str) -> bool {
    Command::new(bin)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
