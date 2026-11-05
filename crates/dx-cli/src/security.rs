//! Comandos de segurança da CLI: `dx audit` e `dx doctor`.
//!
//! Apresentam o veredicto do gate (`dx-security`) de forma legível e devolvem
//! um código de saída coerente com a decisão (0 aprovado, 4 bloqueado,
//! 5 pendente/base expirada).

use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use camino::Utf8Path;
use dx_security::{
    audit, db, inverse_dependency_path, run_gate, Decision, Finding, Policy, Verdict,
};

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `dx audit` — corre o gate e reporta.
pub fn cmd_audit(root: &Utf8Path) -> Result<ExitCode> {
    let policy = Policy::from_env();
    let outcome = run_gate(root, root, &policy, now_unix())
        .context("correr o gate de segurança (cargo-audit instalado?)")?;
    report_verdict(
        &outcome.verdict,
        root,
        outcome.from_cache,
        outcome.deny_covered,
    );
    Ok(ExitCode::from(outcome.verdict.decision.exit_code()))
}

/// Imprime um veredicto de forma legível. `tree_dir` é onde correr
/// `cargo tree -i` para o caminho de dependências (o directório que tem o
/// Cargo.lock avaliado — projecto real ou staging).
pub fn report_verdict(v: &Verdict, tree_dir: &Utf8Path, from_cache: bool, deny_covered: bool) {
    eprintln!("== Gate de segurança DX ==");
    eprintln!(
        "Scanners: cargo-audit (advisories) + cargo-deny {}",
        if deny_covered {
            "(licenças/origens/bans)"
        } else {
            "— AUSENTE: licenças/origens/bans SEM cobertura"
        }
    );
    eprintln!(
        "Base de advisories: {} ({})",
        if v.db_status.valid {
            "válida"
        } else {
            "INVÁLIDA/EXPIRADA"
        },
        v.db_status.detail
    );
    if from_cache {
        eprintln!("(veredicto reutilizado do cache — chave válida; nenhum redownload)");
    }

    if v.findings.is_empty() {
        eprintln!("Achados: nenhum.");
    } else {
        eprintln!("Achados ({}):", v.findings.len());
        for (i, f) in v.findings.iter().enumerate() {
            let blocks = v.blocking.contains(&i);
            print_finding(f, blocks);
            if blocks {
                if let Some(path) = inverse_dependency_path(tree_dir, &f.package, &f.version) {
                    eprintln!("      caminho:");
                    for line in path.lines() {
                        eprintln!("        {line}");
                    }
                }
            }
        }
    }

    eprintln!();
    match v.decision {
        Decision::Approved => {
            eprintln!("Decisão: APROVADO ✓ (grafo sem vulnerabilidades conhecidas e base válida)")
        }
        Decision::Blocked => eprintln!(
            "Decisão: BLOQUEADO ✗ — {} achado(s) a bloquear.",
            v.blocking.len()
        ),
        Decision::Pending => eprintln!(
            "Decisão: PENDENTE — base expirada/indisponível. O download pode terminar, mas o USO não está autorizado até renovar a base (ver `dx doctor`)."
        ),
    }
    eprintln!(
        "Nota honesta: ausência de achado é apenas ausência de vulnerabilidade CONHECIDA nesta base/instante — não é prova de benignidade."
    );
}

fn print_finding(f: &Finding, blocks: bool) {
    let mark = if blocks { "BLOQUEIA" } else { "aviso" };
    let id = f.advisory_id.as_deref().unwrap_or("-");
    eprintln!(
        "  [{}] {} {}@{} ({}) — {}",
        mark,
        f.category.as_str(),
        f.package,
        f.version,
        id,
        f.title
    );
    if let Some(url) = &f.url {
        eprintln!("      {url}");
    }
}

/// `dx doctor` — diagnóstico do ambiente e da base.
pub fn cmd_doctor() -> Result<ExitCode> {
    eprintln!("== dx doctor ==");
    tool_version("rustc", &["--version"]);
    tool_version("cargo", &["--version"]);
    let scanner = audit::scanner_version();
    eprintln!("cargo-audit: {scanner}");
    eprintln!(
        "cargo-deny: {}",
        if dx_security::deny::available() {
            "presente (licenças/origens/bans)"
        } else {
            "AUSENTE — instalar p/ cobrir licenças/origens/bans"
        }
    );

    let policy = Policy::from_env();
    let db_path = db::default_db_path();
    let status = db::evaluate(&db_path, now_unix(), policy.max_db_age_secs);
    eprintln!("advisory-db: {}", status.path);
    eprintln!(
        "  estado: {} — {}",
        if status.valid { "válida" } else { "INVÁLIDA" },
        status.detail
    );
    eprintln!(
        "política (baseline): vulnerability=block, malware=block, integrity=block, source-not-allowed=block, yanked=warn, unmaintained=warn; validade dev={}h",
        policy.max_db_age_secs / 3600
    );
    if !status.valid {
        eprintln!(
            "sugestão: renovar a base com `cargo audit fetch` (ou `git -C {} pull`) e repetir.",
            status.path
        );
        return Ok(ExitCode::from(5));
    }
    Ok(ExitCode::SUCCESS)
}

fn tool_version(bin: &str, args: &[&str]) {
    match Command::new(bin).args(args).output() {
        Ok(o) if o.status.success() => {
            eprintln!("{bin}: {}", String::from_utf8_lossy(&o.stdout).trim())
        }
        _ => eprintln!("{bin}: NÃO ENCONTRADO"),
    }
}
