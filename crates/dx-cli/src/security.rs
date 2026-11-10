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
        .context("run the security gate (is cargo-audit installed?)")?;
    report_verdict(
        &outcome.verdict,
        root,
        outcome.from_cache,
        outcome.from_global,
        outcome.deny_covered,
    );
    Ok(ExitCode::from(outcome.verdict.decision.exit_code()))
}

/// Imprime um veredicto de forma legível. `tree_dir` é onde correr
/// `cargo tree -i` para o caminho de dependências (o directório que tem o
/// Cargo.lock avaliado — projecto real ou staging).
pub fn report_verdict(
    v: &Verdict,
    tree_dir: &Utf8Path,
    from_cache: bool,
    from_global: bool,
    deny_covered: bool,
) {
    eprintln!("== DX security gate ==");
    eprintln!(
        "Scanners: cargo-audit (advisories) + cargo-deny {}",
        if deny_covered {
            "(licenses/sources/bans)"
        } else {
            "— ABSENT: licenses/sources/bans NOT covered"
        }
    );
    eprintln!(
        "Advisory base: {} ({})",
        if v.db_status.valid {
            "valid"
        } else {
            "INVALID/EXPIRED"
        },
        v.db_status.detail
    );
    if from_cache {
        eprintln!(
            "(verdict reused from {} cache — valid key; no re-download)",
            if from_global { "GLOBAL" } else { "local" }
        );
    }

    if v.findings.is_empty() {
        eprintln!("Findings: none.");
    } else {
        eprintln!("Findings ({}):", v.findings.len());
        for (i, f) in v.findings.iter().enumerate() {
            let blocks = v.blocking.contains(&i);
            print_finding(f, blocks);
            if blocks {
                if let Some(path) = inverse_dependency_path(tree_dir, &f.package, &f.version) {
                    eprintln!("      path:");
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
            eprintln!("Decision: APPROVED \u{2713} (graph with no known vulnerabilities and valid base)")
        }
        Decision::Blocked => eprintln!(
            "Decision: BLOCKED \u{2717} — {} blocking finding(s).",
            v.blocking.len()
        ),
        Decision::Pending => eprintln!(
            "Decision: PENDING — base expired/unavailable. The download may finish, but USE is not authorized until the base is renewed (see `dx doctor`)."
        ),
    }
    eprintln!(
        "Honest note: absence of findings is only absence of KNOWN vulnerabilities for this base/instant — not proof of benignity."
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
            "present (licenses/sources/bans)"
        } else {
            "ABSENT — install to cover licenses/sources/bans"
        }
    );

    let caps = dx_sandbox::Capabilities::detect();
    eprintln!(
        "isolation ({}): network={} rlimits={} filesystem={} (FS not enforced in this version)",
        caps.platform, caps.network_isolation, caps.resource_limits, caps.filesystem_isolation
    );

    let policy = Policy::from_env();
    let db_path = db::default_db_path();
    let status = db::evaluate(&db_path, now_unix(), policy.max_db_age_secs);
    eprintln!("advisory-db: {}", status.path);
    eprintln!(
        "  state: {} — {}",
        if status.valid { "valid" } else { "INVALID" },
        status.detail
    );
    eprintln!(
        "policy (baseline): vulnerability=block, malware=block, integrity=block, source-not-allowed=block, yanked=warn, unmaintained=warn; dev validity={}h",
        policy.max_db_age_secs / 3600
    );
    if !status.valid {
        eprintln!(
            "suggestion: renew the base with `cargo audit fetch` (or `git -C {} pull`) and retry.",
            status.path
        );
        return Ok(ExitCode::from(5));
    }
    Ok(ExitCode::SUCCESS)
}

/// `dx fetch` — verifica a integridade do grafo bloqueado (sem compilar).
/// Checksum dos `.crate` em cache vs Cargo.lock; opcionalmente extracção segura.
pub fn cmd_fetch(root: &Utf8Path, locked: bool, extract_check: bool) -> Result<ExitCode> {
    let lockfile = root.join("Cargo.lock");
    if !lockfile.exists() {
        if locked {
            anyhow::bail!("--locked: no Cargo.lock in {root}");
        }
        eprintln!("sem Cargo.lock em {root} — nada para verificar (corre `cargo generate-lockfile` ou `dx add`).");
        return Ok(ExitCode::SUCCESS);
    }

    let limits = dx_integrity::Limits::default();
    let report = dx_integrity::verify_lockfile(&lockfile, &limits, extract_check)
        .context("verificar integridade do lockfile")?;

    eprintln!("== dx fetch: integrity of the locked graph ==");
    eprintln!(
        "Verificados (checksum bate){}: {}",
        if extract_check {
            " + safe extraction"
        } else {
            ""
        },
        report.verified.len()
    );
    if !report.missing.is_empty() {
        eprintln!(
            "Not covered ({} not downloaded — NO known match, never \"intact\"):",
            report.missing.len()
        );
        for m in &report.missing {
            eprintln!("  - {m}");
        }
    }
    for e in &report.mismatched {
        eprintln!("  [BLOCKS] integrity: {e}");
    }
    for (label, e) in &report.extract_failures {
        eprintln!("  [BLOCKS] unsafe extraction in {label}: {e}");
    }

    if report.has_failure() {
        eprintln!(
            "\nDecision: BLOCKED \u{2717} — {} integrity/extraction failure(s). Nothing proceeds to build.",
            report.mismatched.len() + report.extract_failures.len()
        );
        Ok(ExitCode::from(4))
    } else {
        eprintln!(
            "\nDecision: integrity OK for what is cached (checksum proves origin, not benignity)."
        );
        Ok(ExitCode::SUCCESS)
    }
}

/// `dx build --isolated` — compila com isolamento REAL (rede off, ambiente sem
/// segredos, limites de recursos) e reporta o que foi de facto imposto.
pub fn cmd_build_isolated(release: bool) -> Result<ExitCode> {
    use dx_sandbox::{run, Capabilities, SandboxSpec};

    let caps = Capabilities::detect();
    let spec = SandboxSpec::default(); // deny_network + scrub_env + rlimits

    // Rede desligada ⇒ deps têm de estar já obtidas: usamos --offline.
    // Um build isolado é hermético: desligamos qualquer rustc-wrapper global
    // (ex.: sccache) que dependa de rede, senão falha ao tentar contactá-la.
    let mut args = vec![
        "build".to_string(),
        "--offline".to_string(),
        "--config".to_string(),
        "build.rustc-wrapper=\"\"".to_string(),
    ];
    if release {
        args.push("--release".into());
    }
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());

    eprintln!("== dx build --isolated ==");
    if spec.deny_network && !caps.network_isolation {
        eprintln!(
            "WARNING: network isolation REQUESTED but UNAVAILABLE on this host (no user+net namespace). Not faking isolation."
        );
    }

    let outcome = run(&spec, &cargo, &args, &caps).context("run isolated build")?;
    let e = &outcome.enforced;
    eprintln!("Enforced:");
    eprintln!(
        "  network isolated: {} — {}",
        e.network_isolated, e.network_mechanism
    );
    eprintln!(
        "  environment without secrets: {} (passed: {})",
        e.env_scrubbed,
        e.env_kept.join(", ")
    );
    eprintln!("  limits: {}", e.rlimits_applied.join(", "));
    eprintln!(
        "  filesystem isolated: {} — {}",
        e.filesystem_isolated, e.filesystem_note
    );
    Ok(ExitCode::from(outcome.code.unwrap_or(1) as u8))
}

fn tool_version(bin: &str, args: &[&str]) {
    match Command::new(bin).args(args).output() {
        Ok(o) if o.status.success() => {
            eprintln!("{bin}: {}", String::from_utf8_lossy(&o.stdout).trim())
        }
        _ => eprintln!("{bin}: NOT FOUND"),
    }
}
