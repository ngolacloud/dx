//! Comandos de build/perfis LLVM/PGO: `dx build --profile`, `dx bench-build`,
//! `dx pgo`. Delegam no Cargo (backend LLVM da toolchain) com argv estruturado.

use std::process::{Command, ExitCode};

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use dx_build::{builtin, measure, pgo, run_measured, target_cpu_rustflag, Profile};
use dx_manifest::Manifest;

/// Localiza o Cargo.toml (subindo do CWD) e o nome do package.
fn locate() -> Result<(Utf8PathBuf, Utf8PathBuf, Option<String>)> {
    let cwd = std::env::current_dir().context("CWD")?;
    let mut dir = Utf8PathBuf::from_path_buf(cwd).map_err(|_| anyhow::anyhow!("CWD não-UTF8"))?;
    loop {
        let candidate = dir.join("Cargo.toml");
        if candidate.exists() {
            let name = Manifest::load(&candidate)
                .ok()
                .and_then(|m| m.package_name().map(str::to_string));
            return Ok((dir.clone(), candidate, name));
        }
        if !dir.pop() {
            bail!("não encontrei um Cargo.toml a partir do directório actual");
        }
    }
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

/// Caminho do binário produzido por um perfil, se o package tiver um bin óbvio.
fn binary_path(root: &Utf8Path, subdir: &str, pkg: &Option<String>) -> Option<Utf8PathBuf> {
    let name = pkg.as_ref()?;
    let p = root.join("target").join(subdir).join(name);
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

/// `dx build --profile <name> [--native] [--measure]`.
pub fn cmd_build_profile(profile_name: &str, native: bool, do_measure: bool) -> Result<ExitCode> {
    let profile = builtin(profile_name).ok_or_else(|| {
        anyhow::anyhow!(
            "perfil desconhecido `{profile_name}` (usa: dev, release, throughput, size)"
        )
    })?;
    let (root, _manifest, pkg) = locate()?;

    eprintln!("== dx build --profile {} ==", profile.name);
    eprintln!("  {}", profile.description);
    eprintln!("  nota: {}", profile.note);

    let mut args = vec!["build".to_string()];
    args.extend(profile.cargo_args());

    let mut envs: Vec<(String, String)> = Vec::new();
    if native {
        if let Some(flag) = target_cpu_rustflag(true) {
            let existing = std::env::var("RUSTFLAGS").unwrap_or_default();
            envs.push((
                "RUSTFLAGS".into(),
                format!("{existing} {flag}").trim().to_string(),
            ));
            eprintln!(
                "  AVISO: --native usa target-cpu=native — binário só corre nesta microarquitectura; NÃO usar em releases distribuídas."
            );
        }
    }

    if do_measure {
        let m = run_measured("build", &cargo(), &args, &envs, Some(&root))?;
        let mut m = m;
        m.binary_bytes =
            binary_path(&root, profile.target_subdir(), &pkg).and_then(|p| measure::file_size(&p));
        report_measurements(std::slice::from_ref(&m));
        return Ok(ExitCode::from(if m.ok { 0 } else { 1 }));
    }

    let status = run_with_env(&cargo(), &args, &envs, Some(&root))?;
    Ok(ExitCode::from(status as u8))
}

/// `dx bench-build [--profiles a,b,c]`: build LIMPO por perfil, tabela de
/// tempo/memória/tamanho. Clean antes de cada build para medir build limpo.
pub fn cmd_bench_build(profiles_csv: Option<&str>) -> Result<ExitCode> {
    let names: Vec<String> = match profiles_csv {
        Some(csv) => csv.split(',').map(|s| s.trim().to_string()).collect(),
        None => dx_build::all_names()
            .iter()
            .map(|s| s.to_string())
            .collect(),
    };
    let (root, _manifest, pkg) = locate()?;
    if !measure::gnu_time_available() {
        eprintln!("(nota: /usr/bin/time ausente — memória máxima não será medida)");
    }

    let mut results = Vec::new();
    for name in &names {
        let profile: Profile = match builtin(name) {
            Some(p) => p,
            None => {
                eprintln!("perfil desconhecido `{name}` — ignorado");
                continue;
            }
        };
        // Build limpo: limpar antes de medir.
        let _ = Command::new(cargo())
            .arg("clean")
            .current_dir(root.as_std_path())
            .status();
        let mut args = vec!["build".to_string()];
        args.extend(profile.cargo_args());
        eprintln!("→ a medir perfil `{name}` (build limpo)...");
        let mut m = run_measured(name, &cargo(), &args, &[], Some(&root))?;
        m.binary_bytes =
            binary_path(&root, profile.target_subdir(), &pkg).and_then(|p| measure::file_size(&p));
        results.push(m);
    }
    report_measurements(&results);
    Ok(ExitCode::SUCCESS)
}

fn report_measurements(ms: &[measure::Measurement]) {
    eprintln!();
    eprintln!(
        "{:<12} {:>8} {:>12} {:>12}  binário",
        "perfil", "ok", "tempo(s)", "mem(MiB)"
    );
    for m in ms {
        let mem = m
            .max_rss_kb
            .map(|k| format!("{:.1}", k as f64 / 1024.0))
            .unwrap_or_else(|| "-".into());
        let bin = m
            .binary_bytes
            .map(|b| format!("{:.2} MiB", b as f64 / (1024.0 * 1024.0)))
            .unwrap_or_else(|| "-".into());
        eprintln!(
            "{:<12} {:>8} {:>12.2} {:>12}  {}",
            m.label,
            if m.ok { "sim" } else { "FALHA" },
            m.wall_secs,
            mem,
            bin
        );
    }
    eprintln!(
        "(tempo = relógio; mem = pico RSS via /usr/bin/time; binário = tamanho do artefacto)"
    );
}

/// `dx pgo [--bin <nome>] [-- <workload args>]`: instrumentar → workload →
/// merge → usar perfil → comparar tamanhos.
pub fn cmd_pgo(bin: Option<&str>, workload: &[String]) -> Result<ExitCode> {
    let (root, _manifest, pkg) = locate()?;
    let bin_name = bin.map(str::to_string).or_else(|| pkg.clone());
    let bin_name = match bin_name {
        Some(n) => n,
        None => bail!("não consegui determinar o binário; usa `--bin <nome>`"),
    };

    if pgo::profdata_tool().is_none() {
        bail!(
            "llvm-profdata não encontrado. Instala `rustup component add llvm-tools-preview` ou garante /usr/bin/llvm-profdata com a MESMA versão de LLVM do rustc."
        );
    }
    let tool = pgo::profdata_tool().unwrap();
    eprintln!("== dx pgo (bin: {bin_name}) ==");
    eprintln!("  llvm-profdata: {tool}");

    let raw_dir = root.join("target/pgo/raw");
    let _ = std::fs::remove_dir_all(raw_dir.as_std_path());
    std::fs::create_dir_all(raw_dir.as_std_path()).context("criar dir de perfis")?;
    let merged = root.join("target/pgo/merged.profdata");

    // 1) Instrumentar (release).
    eprintln!("1/4 a compilar instrumentado (-Cprofile-generate)...");
    let gen_flags = pgo::rustflags_generate(&raw_dir);
    let status = run_with_env(
        &cargo(),
        &["build".into(), "--release".into()],
        &[("RUSTFLAGS".into(), gen_flags)],
        Some(&root),
    )?;
    if status != 0 {
        bail!("build instrumentado falhou");
    }

    // 2) Correr o workload para gerar .profraw.
    let bin_path = root.join("target/release").join(&bin_name);
    if !bin_path.exists() {
        bail!("binário instrumentado não encontrado em {bin_path} (é um bin?)");
    }
    eprintln!("2/4 a correr o workload para gerar perfis...");
    let (wprog, wargs) = if workload.is_empty() {
        (bin_path.to_string(), Vec::new())
    } else {
        (workload[0].clone(), workload[1..].to_vec())
    };
    let _ = run_with_env(&wprog, &wargs, &[], Some(&root));
    let size_instrumented = measure::file_size(&bin_path);

    // 3) Fundir perfis.
    eprintln!("3/4 a fundir perfis (llvm-profdata merge)...");
    let n = pgo::merge(&raw_dir, &merged).context("fusão de perfis PGO")?;
    eprintln!("  {n} ficheiro(s) .profraw fundido(s) → {merged}");

    // 4) Recompilar usando o perfil.
    eprintln!("4/4 a recompilar com o perfil (-Cprofile-use)...");
    let use_flags = pgo::rustflags_use(&merged);
    let status = run_with_env(
        &cargo(),
        &["build".into(), "--release".into()],
        &[("RUSTFLAGS".into(), use_flags)],
        Some(&root),
    )?;
    if status != 0 {
        bail!("build com perfil falhou (perfil incompatível com o código/toolchain?)");
    }
    let size_optimized = measure::file_size(&bin_path);

    eprintln!();
    eprintln!("PGO concluído. Tamanho do binário:");
    eprintln!(
        "  instrumentado: {}",
        size_instrumented
            .map(fmt_bytes)
            .unwrap_or_else(|| "-".into())
    );
    eprintln!(
        "  com perfil:    {}",
        size_optimized.map(fmt_bytes).unwrap_or_else(|| "-".into())
    );
    eprintln!(
        "(para ganho de performance, medir o serviço com uma carga representativa — ver inc.3/§9)"
    );
    Ok(ExitCode::SUCCESS)
}

fn fmt_bytes(b: u64) -> String {
    format!("{:.2} MiB", b as f64 / (1024.0 * 1024.0))
}

/// Corre um comando com env extra, herdando stdio; devolve o código de saída.
fn run_with_env(
    program: &str,
    args: &[String],
    envs: &[(String, String)],
    cwd: Option<&Utf8Path>,
) -> Result<i32> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    if let Some(c) = cwd {
        cmd.current_dir(c.as_std_path());
    }
    let status = cmd.status().with_context(|| format!("invocar {program}"))?;
    Ok(status.code().unwrap_or(1))
}
