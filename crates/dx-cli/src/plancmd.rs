//! `dx add --plan` — "o que estou a introduzir no meu projecto?"
//!
//! Calcula o IMPACTO de adicionar uma dependência, sobre o grafo candidato
//! resolvido em staging (sem aterrar, sem compilar, sem correr build.rs):
//! packages novos (directos/transitivos), superfícies de EXECUÇÃO
//! (`build.rs`/proc-macro/nativo), MSRV, veredicto de segurança, mudança no
//! manifesto e download (cache vs novo). Números só quando medidos; nunca
//! inventados.

use std::collections::BTreeMap;
use std::process::{Command, ExitCode};

use anyhow::{Context, Result};
use camino::Utf8Path;
use dx_core::DepSpec;
use dx_integrity::{all_packages, cached_crate_path, diff_added, PackageEntry};
use dx_security::{run_gate, Decision, Policy};

use crate::{security, stage};

#[derive(Default, Clone)]
struct PkgAttrs {
    build_script: bool,
    proc_macro: bool,
    links: Option<String>,
    rust_version: Option<String>,
}

/// Imprime o plano de impacto. Não aterra nada.
pub fn impact(
    root: &Utf8Path,
    manifest_path: &Utf8Path,
    mutated_toml: &str,
    spec: &DepSpec,
    offline: bool,
) -> Result<ExitCode> {
    eprintln!("== dx add --plan: {} ==", spec.table_entry_key());
    eprintln!(
        "Manifest change ({manifest_path}): {} = {} [{}]",
        spec.table_entry_key(),
        spec.version_req.as_deref().unwrap_or("*"),
        spec.source
    );

    let before = all_packages(&root.join("Cargo.lock")).unwrap_or_default();

    match stage::resolve_candidate(root, mutated_toml, offline) {
        stage::Staging::Resolved { root: staging, .. } => {
            let after = all_packages(&staging.join("Cargo.lock")).unwrap_or_default();
            // Exclude the root package itself (it is not a dependency being
            // introduced; it shows up only when there was no prior lockfile).
            let root_pkg = dx_manifest::Manifest::load(manifest_path)
                .ok()
                .and_then(|m| m.package_name().map(str::to_string));
            let added: Vec<PackageEntry> = diff_added(&before, &after)
                .into_iter()
                .filter(|p| Some(&p.name) != root_pkg.as_ref())
                .collect();
            let attrs = metadata_attrs(&staging);

            print_graph(spec, &added, &attrs);
            print_execution_surfaces(&added, &attrs);
            print_msrv(&added, &attrs);
            print_download(&added);

            // Segurança: corre o gate sobre o grafo candidato.
            let policy = Policy::from_env();
            let outcome = run_gate(root, &staging, &policy, security::now_unix())
                .context("gate over the candidate graph")?;
            let v = &outcome.verdict;
            eprintln!("\nSecurity (candidate graph):");
            eprintln!(
                "  scanners: cargo-audit{}",
                if outcome.deny_covered {
                    " + cargo-deny"
                } else {
                    " (cargo-deny absent)"
                }
            );
            if v.findings.is_empty() {
                eprintln!("  findings: none known (NOT proof of benignity).");
            } else {
                eprintln!(
                    "  findings: {} (blocking: {})",
                    v.findings.len(),
                    v.blocking.len()
                );
                for (i, f) in v.findings.iter().enumerate() {
                    if i == 6 {
                        eprintln!("    \u{2026} and {} more", v.findings.len() - 6);
                        break;
                    }
                    eprintln!(
                        "    [{}] {}@{} — {}",
                        f.category.as_str(),
                        f.package,
                        f.version,
                        f.title
                    );
                }
            }
            let decision = match v.decision {
                Decision::Approved => "APPROVED (would land)",
                Decision::Blocked => "BLOCKED (would not land)",
                Decision::Pending => "PENDING (base expired; would not land)",
            };
            eprintln!("  decision: {decision} — {}", v.db_status.detail);

            eprintln!("\n(plan: nothing was written. To apply: rerun without --plan.)");
            Ok(ExitCode::SUCCESS)
        }
        stage::Staging::NotApplicable { reason } => {
            eprintln!(
                "\nGraph not analyzable for this project shape ({reason}).\n\
                 The plan covers the manifest change above; run `dx audit` after landing \
                 to evaluate the graph. (Full impact requires a self-contained manifest.)"
            );
            Ok(ExitCode::from(6))
        }
        stage::Staging::ResolveFailed { reason } => {
            eprintln!("\nerror: could not resolve the candidate graph:\n  {reason}");
            Ok(ExitCode::FAILURE)
        }
    }
}

fn print_graph(spec: &DepSpec, added: &[PackageEntry], _attrs: &BTreeMap<String, PkgAttrs>) {
    if added.is_empty() {
        eprintln!("\nGraph: no new package (already present or no new transitives).");
        return;
    }
    let direct = &spec.name;
    eprintln!("\nNEW packages in the graph ({}):", added.len());
    for p in added {
        let kind = if &p.name == direct {
            "direct"
        } else {
            "transitive"
        };
        eprintln!("  + {}@{} ({kind})", p.name, p.version);
    }
}

fn print_execution_surfaces(added: &[PackageEntry], attrs: &BTreeMap<String, PkgAttrs>) {
    let mut build_rs = Vec::new();
    let mut procm = Vec::new();
    let mut native = Vec::new();
    for p in added {
        if let Some(a) = attrs.get(&key(&p.name, &p.version)) {
            if a.build_script {
                build_rs.push(format!("{}@{}", p.name, p.version));
            }
            if a.proc_macro {
                procm.push(format!("{}@{}", p.name, p.version));
            }
            if let Some(l) = &a.links {
                native.push(format!("{}@{} (links={l})", p.name, p.version));
            }
        }
    }
    eprintln!("\nEXECUTION surfaces during the build (review with care):");
    eprintln!("  build.rs:     {}", list_or_none(&build_rs));
    eprintln!("  proc-macros:  {}", list_or_none(&procm));
    eprintln!("  native/links: {}", list_or_none(&native));
}

fn print_msrv(added: &[PackageEntry], attrs: &BTreeMap<String, PkgAttrs>) {
    let mut max: Option<String> = None;
    for p in added {
        if let Some(a) = attrs.get(&key(&p.name, &p.version)) {
            if let Some(rv) = &a.rust_version {
                if max.as_deref().map(|m| version_lt(m, rv)).unwrap_or(true) {
                    max = Some(rv.clone());
                }
            }
        }
    }
    match max {
        Some(rv) => eprintln!("\nMSRV required by new packages: >= {rv}"),
        None => eprintln!("\nMSRV: no new package declares rust-version."),
    }
}

fn print_download(added: &[PackageEntry]) {
    let (mut cached, mut fresh) = (0usize, 0usize);
    for p in added {
        let is_registry = p
            .source
            .as_deref()
            .map(|s| s.starts_with("registry+"))
            .unwrap_or(false);
        if !is_registry {
            continue;
        }
        if cached_crate_path(&p.name, &p.version).is_some() {
            cached += 1;
        } else {
            fresh += 1;
        }
    }
    eprintln!(
        "\nDownload: {cached} already cached, {fresh} new to download \
         (sizes not estimated — we do not invent bytes)."
    );
}

fn list_or_none(v: &[String]) -> String {
    if v.is_empty() {
        "none".into()
    } else {
        v.join(", ")
    }
}

fn key(name: &str, version: &str) -> String {
    format!("{name}@{version}")
}

/// Atributos por package a partir de `cargo metadata` (com deps) do staging.
fn metadata_attrs(staging: &Utf8Path) -> BTreeMap<String, PkgAttrs> {
    let mut map = BTreeMap::new();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args(["metadata", "--format-version", "1"])
        .current_dir(staging.as_std_path())
        .output();
    let out = match out {
        Ok(o) if o.status.success() => o.stdout,
        _ => return map,
    };
    let v: serde_json::Value = match serde_json::from_slice(&out) {
        Ok(v) => v,
        Err(_) => return map,
    };
    if let Some(pkgs) = v.get("packages").and_then(|p| p.as_array()) {
        for p in pkgs {
            let name = p.get("name").and_then(|x| x.as_str()).unwrap_or("");
            let version = p.get("version").and_then(|x| x.as_str()).unwrap_or("");
            let mut a = PkgAttrs {
                links: p.get("links").and_then(|x| x.as_str()).map(str::to_string),
                rust_version: p
                    .get("rust_version")
                    .and_then(|x| x.as_str())
                    .map(str::to_string),
                ..Default::default()
            };
            if let Some(targets) = p.get("targets").and_then(|t| t.as_array()) {
                for t in targets {
                    if let Some(kinds) = t.get("kind").and_then(|k| k.as_array()) {
                        for k in kinds {
                            match k.as_str() {
                                Some("custom-build") => a.build_script = true,
                                Some("proc-macro") => a.proc_macro = true,
                                _ => {}
                            }
                        }
                    }
                }
            }
            map.insert(key(name, version), a);
        }
    }
    map
}

/// `a < b` para versões "x.y.z" (comparação numérica simples).
fn version_lt(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> { s.split('.').map(|x| x.parse().unwrap_or(0)).collect() };
    parse(a) < parse(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_lt_numeric() {
        assert!(version_lt("1.70", "1.74"));
        assert!(version_lt("1.9", "1.10"));
        assert!(!version_lt("1.74", "1.70"));
    }
}
