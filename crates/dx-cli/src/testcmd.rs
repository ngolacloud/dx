//! `dx test` — corre uma suite (configurável) e emite relatório versionado.

use std::process::{Command, ExitCode};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use dx_report::{
    eval_load, parse_libtest, resolve_suite, Case, Format, LoadMetrics, Report, Status, Suite,
    SuiteConfig, SuiteKind,
};

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
        .unwrap_or_else(|| "unknown".into())
}

pub fn cmd_test(
    suite_name: Option<&str>,
    format: Option<&str>,
    out: Option<&Utf8Path>,
    no_ansi: bool,
) -> Result<ExitCode> {
    let root = locate_root()?;
    let name = suite_name.unwrap_or("unit");
    let fmt = match format {
        Some(f) => Format::parse(f)
            .ok_or_else(|| anyhow::anyhow!("invalid format `{f}` (terminal|json|junit|html)"))?,
        None => Format::Terminal,
    };

    // Config: dx.toml do projecto (precedência) ou builtin.
    let dx_toml = std::fs::read_to_string(root.join("dx.toml").as_std_path()).ok();
    let cfg = resolve_suite(name, dx_toml.as_deref())
        .map_err(|e| anyhow::anyhow!("invalid dx.toml: {e}"))?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "suite `{name}` is not classified: define it in dx.toml ([test.suites.{name}]) \
                 or use unit/doc/integration (we do not classify suites by name without config)"
            )
        })?;

    let suite = run_suite(&root, &cfg)?;
    let mut report = Report::new(now_iso());
    report.suites.push(suite);

    let rendered = fmt.render(&report, !no_ansi && out.is_none());
    match out {
        Some(path) => {
            std::fs::write(path.as_std_path(), rendered.as_bytes())
                .with_context(|| format!("escrever {path}"))?;
            eprintln!("{} report written to {path}", fmt_label(fmt));
        }
        None => println!("{rendered}"),
    }

    Ok(if report.ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn fmt_label(f: Format) -> &'static str {
    match f {
        Format::Terminal => "terminal",
        Format::Json => "JSON",
        Format::Junit => "JUnit",
        Format::Html => "HTML",
    }
}

fn run_suite(root: &Utf8Path, cfg: &SuiteConfig) -> Result<Suite> {
    if cfg.runner.is_empty() {
        bail!("suite `{}` has no runner", cfg.name);
    }
    let (prog, args) = (&cfg.runner[0], &cfg.runner[1..]);
    let start = Instant::now();
    let output = Command::new(prog)
        .args(args)
        .current_dir(root.as_std_path())
        .output()
        .with_context(|| format!("invoke runner `{prog}`"))?;
    let dur = start.elapsed().as_millis() as u64;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let runner_ok = output.status.success();

    let mut suite = Suite {
        name: cfg.name.clone(),
        runner: cfg.runner.clone(),
        cases: Vec::new(),
        duration_ms: Some(dur),
        ok: false,
        runner_error: None,
    };

    match cfg.kind {
        SuiteKind::Rust => {
            suite.cases = parse_libtest(&stdout);
            // Erro de runner (ex.: falha de compilação) sem casos → não verde.
            if !runner_ok && suite.cases.is_empty() {
                suite.runner_error = Some(tail(&stderr));
            }
            suite.ok = runner_ok && suite.failed() == 0;
        }
        SuiteKind::Command => {
            suite.cases.push(Case {
                name: cfg.name.clone(),
                status: if runner_ok {
                    Status::Passed
                } else {
                    Status::Failed
                },
                duration_ms: Some(dur),
                message: if runner_ok { None } else { Some(tail(&stderr)) },
            });
            suite.ok = runner_ok;
        }
        SuiteKind::Load => {
            // Métricas: ficheiro JSON indicado por DX_LOAD_METRICS ou
            // target/dx-load-metrics.json, escrito pelo gerador.
            let metrics_path = std::env::var("DX_LOAD_METRICS")
                .map(Utf8PathBuf::from)
                .unwrap_or_else(|_| root.join("target/dx-load-metrics.json"));
            let thresholds = cfg.load.clone().unwrap_or_default();
            match std::fs::read_to_string(metrics_path.as_std_path()) {
                Ok(text) => {
                    let metrics: LoadMetrics = serde_json::from_str(&text)
                        .with_context(|| format!("invalid metrics in {metrics_path}"))?;
                    let violations = eval_load(&thresholds, &metrics);
                    if violations.is_empty() {
                        suite.cases.push(Case {
                            name: format!("{}:thresholds", cfg.name),
                            status: Status::Passed,
                            duration_ms: Some(dur),
                            message: None,
                        });
                    } else {
                        for v in violations {
                            suite.cases.push(Case {
                                name: format!(
                                    "{}:{}",
                                    cfg.name,
                                    v.split(' ').next().unwrap_or("threshold")
                                ),
                                status: Status::Failed,
                                duration_ms: None,
                                message: Some(v),
                            });
                        }
                    }
                    // Carga falha por thresholds E por erro do gerador.
                    suite.ok = runner_ok && suite.failed() == 0;
                }
                Err(_) => {
                    suite.cases.push(Case {
                        name: format!("{}:thresholds", cfg.name),
                        status: Status::NotRun,
                        duration_ms: None,
                        message: Some(format!(
                            "no metrics in {metrics_path} (the generator must write them)"
                        )),
                    });
                    suite.runner_error = Some(format!("no load metrics in {metrics_path}"));
                    suite.ok = false;
                }
            }
        }
    }
    Ok(suite)
}

fn tail(s: &str) -> String {
    let s = s.trim();
    if s.len() > 1200 {
        format!("...{}", &s[s.len() - 1200..])
    } else {
        s.to_string()
    }
}
