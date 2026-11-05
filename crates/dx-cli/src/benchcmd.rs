//! `dx bench` — benchmark do pipeline de dependências (§9).
//!
//! Mede resolução (cargo) vs análise/gate (DX) sobre um mock registry
//! determinista, com cache de veredicto frio/quente, p50/p95 e memória.

use std::process::{Command, ExitCode};
use std::time::Instant;

use anyhow::{Context, Result};
use camino::Utf8Path;
use dx_bench::{gen_mock_workspace, SizeResult, Stats};
use dx_security::{run_gate, Policy};

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

fn now_unix() -> u64 {
    crate::security::now_unix()
}

/// Corre `cargo generate-lockfile` (resolução) uma vez; devolve ms.
fn resolve_once(root: &Utf8Path) -> Result<f64> {
    let _ = std::fs::remove_file(root.join("Cargo.lock").as_std_path()); // cold
    let start = Instant::now();
    let status = Command::new(cargo())
        .args(["generate-lockfile", "--offline"])
        .current_dir(root.as_std_path())
        .output()
        .context("generate-lockfile")?;
    if !status.status.success() {
        // mock por path não precisa de rede; se falhar, reportar.
        anyhow::bail!(
            "resolução falhou: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }
    Ok(start.elapsed().as_secs_f64() * 1000.0)
}

/// Corre o gate uma vez; devolve ms. `clear_cache` força cache miss (frio).
fn gate_once(root: &Utf8Path, policy: &Policy, clear_cache: bool) -> Result<f64> {
    if clear_cache {
        let _ = std::fs::remove_dir_all(root.join(".dx/security").as_std_path());
    }
    let start = Instant::now();
    let _ = run_gate(root, root, policy, now_unix()).context("gate")?;
    Ok(start.elapsed().as_secs_f64() * 1000.0)
}

pub fn cmd_bench(sizes_csv: Option<&str>, reps: usize) -> Result<ExitCode> {
    let sizes: Vec<usize> = match sizes_csv {
        Some(csv) => csv
            .split(',')
            .filter_map(|s| s.trim().parse::<usize>().ok())
            .collect(),
        None => vec![10, 100],
    };
    let reps = reps.max(1);
    let policy = Policy::from_env();

    eprintln!("== dx bench (mock registry determinista; {reps} repetições) ==");
    eprintln!(
        "Cobertura: resolução + gate (cache frio/quente). FORA: download real, rede rápida/lenta, amostra real (grafo por path não descarrega)."
    );

    let mut results = Vec::new();
    for &n in &sizes {
        eprintln!("\n→ a gerar mock-workspace de {n} packages...");
        let tmp = tempfile::tempdir().context("temp")?;
        let root = Utf8Path::from_path(tmp.path()).context("temp não-UTF8")?;
        gen_mock_workspace(root, n).context("gerar mock")?;

        // Resolução (sempre fria: lockfile removido a cada rep).
        let mut resolve = Vec::new();
        for _ in 0..reps {
            resolve.push(resolve_once(root)?);
        }
        // Garantir um lockfile para o gate.
        resolve_once(root)?;

        // Gate frio: limpar cache de veredicto a cada rep.
        let mut gate_cold = Vec::new();
        for _ in 0..reps {
            gate_cold.push(gate_once(root, &policy, true)?);
        }
        // Gate quente: aquecer uma vez, depois medir hits de cache.
        gate_once(root, &policy, false)?;
        let mut gate_warm = Vec::new();
        for _ in 0..reps {
            gate_warm.push(gate_once(root, &policy, false)?);
        }

        let res = SizeResult {
            packages: n,
            resolve: Stats::from_samples(&resolve),
            gate_cold: Stats::from_samples(&gate_cold),
            gate_warm: Stats::from_samples(&gate_warm),
            max_rss_kb: None,
            warm_overhead_pct: 0.0,
        }
        .with_overhead();
        results.push(res);
    }

    // Tabela.
    eprintln!(
        "\n{:>6} | {:>18} | {:>18} | {:>18} | {:>12}",
        "pkgs", "resolve p50/p95", "gate-frio p50/p95", "gate-quente p50/p95", "overhead"
    );
    for r in &results {
        eprintln!(
            "{:>6} | {:>8.1}/{:<8.1} | {:>8.1}/{:<8.1} | {:>8.1}/{:<8.1} | {:>10.1}%",
            r.packages,
            r.resolve.p50,
            r.resolve.p95,
            r.gate_cold.p50,
            r.gate_cold.p95,
            r.gate_warm.p50,
            r.gate_warm.p95,
            r.warm_overhead_pct,
        );
    }
    eprintln!("(tempos em ms; overhead = gate-quente p50 / resolução p50)");
    eprintln!(
        "Meta de engenharia: ≤10% de overhead em cache quente — OBJECTIVO, não garantia. As verificações NUNCA são desligadas para a cumprir."
    );
    let worst = results
        .iter()
        .map(|r| r.warm_overhead_pct)
        .fold(0.0_f64, f64::max);
    eprintln!(
        "Overhead quente máximo medido: {worst:.1}% ({}).",
        if worst <= 10.0 {
            "dentro da meta"
        } else {
            "acima da meta — rever"
        }
    );

    Ok(ExitCode::SUCCESS)
}
