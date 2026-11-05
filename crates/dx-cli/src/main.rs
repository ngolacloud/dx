//! `dx` — CLI fina sobre bibliotecas testáveis (dx-core, dx-manifest,
//! dx-security, dx-integrity, dx-sandbox, dx-build, dx-release, dx-report,
//! dx-seiva).
//!
//! Comandos implementados: `new`/`init` (inc. `--template seiva`), `add`
//! (idempotente + gate de segurança + adaptador Seiva), `remove`, `build`
//! (`--profile`/`--isolated`/`--measure`), `bench-build`, `pgo`, `test`
//! (suites configuráveis + relatórios), `audit`, `doctor`, `fetch`,
//! `release prepare`. `update` fica como stub honesto (exit 3).

mod benchcmd;
mod buildcmd;
mod plancmd;
mod releasecmd;
mod scaffold;
mod security;
mod seivacmd;
mod stage;
mod suspect;
mod testcmd;
mod updatecmd;

use std::process::{Command, ExitCode};

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use clap::{Args, Parser, Subcommand};
use dx_core::{Change, DepKind, DepSpec, Plan, Source};
use dx_manifest::{apply_transaction, commit_manifest, recover, FileWrite, Manifest};
use dx_security::{run_gate, Decision, Policy};

#[derive(Parser)]
#[command(
    name = "dx",
    about = "DX — explainable, security-first, Cargo-compatible Rust dependency management (binary: dx)",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new project (`--lib`, `--bin` or `--template seiva`).
    New {
        name: String,
        #[arg(long)]
        lib: bool,
        #[arg(long)]
        bin: bool,
        /// Template: seiva (platform with a managed composition).
        #[arg(long)]
        template: Option<String>,
        #[command(flatten)]
        dry: DryRun,
    },
    /// Initialize a project in the current directory.
    Init {
        #[arg(long)]
        lib: bool,
        #[arg(long)]
        bin: bool,
        #[arg(long)]
        template: Option<String>,
        #[command(flatten)]
        dry: DryRun,
    },
    /// Add a dependency (idempotent).
    Add {
        /// `name` or `name@requirement`.
        spec: String,
        #[arg(long, value_delimiter = ',')]
        features: Vec<String>,
        #[arg(long)]
        no_default_features: bool,
        #[arg(long)]
        optional: bool,
        #[arg(long)]
        dev: bool,
        #[arg(long)]
        build: bool,
        #[arg(long)]
        git: Option<String>,
        #[arg(long)]
        rev: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        registry: Option<String>,
        #[arg(long)]
        manifest_path: Option<Utf8PathBuf>,
        /// Resolve the candidate graph without network (Cargo cache only).
        #[arg(long)]
        offline: bool,
        /// Show the IMPACT (graph, execution, MSRV, security) without landing.
        #[arg(long)]
        plan: bool,
        #[command(flatten)]
        dry: DryRun,
    },
    /// Remove a dependency (idempotent).
    Remove {
        name: String,
        #[arg(long)]
        dev: bool,
        #[arg(long)]
        build: bool,
        #[arg(long)]
        manifest_path: Option<Utf8PathBuf>,
        #[command(flatten)]
        dry: DryRun,
    },
    /// Format (delegates to `cargo fmt`).
    Fmt {
        #[arg(long)]
        check: bool,
    },
    /// Lint (delegates to `cargo clippy`).
    Lint,
    /// Check without building the final binary (delegates to `cargo check`).
    Check,
    /// Build (delegates to `cargo build`).
    Build {
        #[arg(long)]
        release: bool,
        /// LLVM profile: dev | release | throughput | size.
        #[arg(long)]
        profile: Option<String>,
        /// target-cpu=native (NOT for distributed releases).
        #[arg(long)]
        native: bool,
        /// Measure build time/memory/size.
        #[arg(long)]
        measure: bool,
        /// Build with real isolation (network off, environment without
        /// secrets, resource limits). Uses `--offline` (deps already fetched).
        #[arg(long)]
        isolated: bool,
    },
    /// Measure a clean build per profile (time/memory/size).
    BenchBuild {
        /// Comma-separated list of profiles (default: all).
        #[arg(long)]
        profiles: Option<String>,
    },
    /// Benchmark the dependency pipeline (resolution vs gate; §9).
    Bench {
        /// Sizes (number of packages), comma-separated (default: 10,100).
        #[arg(long)]
        sizes: Option<String>,
        /// Repetitions per measurement (default: 5).
        #[arg(long, default_value_t = 5)]
        reps: usize,
    },
    /// PGO: instrument -> workload -> merge profiles -> rebuild -> compare.
    Pgo {
        #[arg(long)]
        bin: Option<String>,
        /// Workload command (default: run the binary itself).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        workload: Vec<String>,
    },
    /// Run (delegates to `cargo run`).
    Run {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run a (configurable) test suite and emit a versioned report.
    Test {
        /// Suite: unit | doc | integration | <defined in dx.toml>.
        #[arg(long)]
        suite: Option<String>,
        /// Report format: terminal | json | junit | html.
        #[arg(long)]
        format: Option<String>,
        /// Write the report to a file instead of stdout.
        #[arg(long)]
        out: Option<Utf8PathBuf>,
        /// Disable terminal colors.
        #[arg(long)]
        no_ansi: bool,
    },
    /// Explain why a crate is in the graph (inverse tree).
    Why {
        /// Crate name (e.g. `openssl-sys`).
        spec: String,
    },
    /// Explain the graph: duplicates or a feature's activation.
    Explain {
        /// Show duplicate versions of the same crate.
        #[arg(long)]
        duplicates: bool,
        /// Who pulls this feature (inverse tree by feature).
        #[arg(long)]
        feature: Option<String>,
    },
    /// Security audit of the graph.
    Audit,
    /// Diagnose the environment.
    Doctor,
    /// Update dependencies: explain, security-fix, apply (with recovery).
    Update {
        /// Restrict the update to one package (minimal change).
        name: Option<String>,
        /// Show what would change, without touching the lockfile.
        #[arg(long)]
        plan: bool,
        /// Report which advisories the update clears / that remain.
        #[arg(long)]
        security: bool,
        /// Apply the update, validate with the gate, restore on failure.
        #[arg(long)]
        apply: bool,
        /// With --apply, also run `cargo test` as validation.
        #[arg(long)]
        test: bool,
        /// Compute the update without network (Cargo cache only).
        #[arg(long)]
        offline: bool,
    },
    /// Verify the integrity of the locked graph (without compiling).
    Fetch {
        /// Require an existing Cargo.lock (fails if absent).
        #[arg(long)]
        locked: bool,
        /// Beyond the checksum, unpack each `.crate` with safeguards
        /// (traversal/symlink/zip-bomb) into a temp dir and discard.
        #[arg(long)]
        extract_check: bool,
    },
    /// Prepare a release.
    Release {
        #[command(subcommand)]
        what: ReleaseCmd,
    },
}

#[derive(Subcommand)]
enum ReleaseCmd {
    Prepare,
}

#[derive(Args, Clone, Copy)]
struct DryRun {
    /// Show the exact plan without changing files.
    #[arg(long)]
    dry_run: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cmd: Commands) -> Result<ExitCode> {
    match cmd {
        Commands::New {
            name,
            lib,
            bin,
            template,
            dry,
        } => cmd_new(&name, lib, bin, template.as_deref(), dry.dry_run),
        Commands::Init {
            lib,
            bin,
            template,
            dry,
        } => cmd_init(lib, bin, template.as_deref(), dry.dry_run),
        Commands::Add {
            spec,
            features,
            no_default_features,
            optional,
            dev,
            build,
            git,
            rev,
            tag,
            branch,
            path,
            registry,
            manifest_path,
            offline,
            plan,
            dry,
        } => cmd_add(AddArgs {
            spec,
            features,
            no_default_features,
            optional,
            dev,
            build,
            git,
            rev,
            tag,
            branch,
            path,
            registry,
            manifest_path,
            offline,
            plan,
            dry_run: dry.dry_run,
        }),
        Commands::Remove {
            name,
            dev,
            build,
            manifest_path,
            dry,
        } => cmd_remove(&name, dev, build, manifest_path, dry.dry_run),
        Commands::Fmt { check } => {
            let mut a = vec!["fmt".to_string()];
            if check {
                a.push("--".into());
                a.push("--check".into());
            }
            cargo_passthrough(&a)
        }
        Commands::Lint => cargo_passthrough(&["clippy".into()]),
        Commands::Check => cargo_passthrough(&["check".into()]),
        Commands::Build {
            release,
            profile,
            native,
            measure,
            isolated,
        } => {
            if isolated {
                return security::cmd_build_isolated(release);
            }
            if let Some(name) = profile {
                return buildcmd::cmd_build_profile(&name, native, measure);
            }
            if native || measure {
                // --native/--measure implicam o perfil release equilibrado.
                return buildcmd::cmd_build_profile("release", native, measure);
            }
            let mut a = vec!["build".to_string()];
            if release {
                a.push("--release".into());
            }
            cargo_passthrough(&a)
        }
        Commands::BenchBuild { profiles } => buildcmd::cmd_bench_build(profiles.as_deref()),
        Commands::Bench { sizes, reps } => benchcmd::cmd_bench(sizes.as_deref(), reps),
        Commands::Pgo { bin, workload } => buildcmd::cmd_pgo(bin.as_deref(), &workload),
        Commands::Run { args } => {
            let mut a = vec!["run".to_string()];
            if !args.is_empty() {
                a.push("--".into());
                a.extend(args);
            }
            cargo_passthrough(&a)
        }
        Commands::Test {
            suite,
            format,
            out,
            no_ansi,
        } => testcmd::cmd_test(suite.as_deref(), format.as_deref(), out.as_deref(), no_ansi),
        Commands::Why { spec } => {
            eprintln!("== dx why {spec} == (who pulls this crate; confirmed by Cargo resolution)");
            cargo_passthrough(&[
                "tree".into(),
                "-i".into(),
                spec,
                "-e".into(),
                "normal,build".into(),
            ])
        }
        Commands::Explain {
            duplicates,
            feature,
        } => {
            if let Some(f) = feature {
                eprintln!("== dx explain --feature {f} == (who activates this feature)");
                cargo_passthrough(&[
                    "tree".into(),
                    "-e".into(),
                    "features".into(),
                    "-i".into(),
                    f,
                ])
            } else if duplicates {
                eprintln!("== dx explain --duplicates == (multiple versions of the same crate)");
                cargo_passthrough(&["tree".into(), "-d".into()])
            } else {
                bail!("use `--duplicates` or `--feature <name>`");
            }
        }
        Commands::Audit => {
            let (root, manifest_path) = locate_manifest(None)?;
            let _ = manifest_path;
            security::cmd_audit(&root)
        }
        Commands::Doctor => security::cmd_doctor(),
        Commands::Update {
            name,
            plan,
            security,
            apply,
            test,
            offline,
        } => updatecmd::cmd_update(name.as_deref(), plan, security, apply, offline, test),
        Commands::Fetch {
            locked,
            extract_check,
        } => {
            let (root, manifest_path) = locate_manifest(None)?;
            let _ = manifest_path;
            security::cmd_fetch(&root, locked, extract_check)
        }
        Commands::Release { what } => match what {
            ReleaseCmd::Prepare => releasecmd::cmd_release_prepare(),
        },
    }
}

// ---------------------------------------------------------------------------
// new / init
// ---------------------------------------------------------------------------

fn resolve_template(lib: bool, bin: bool, template: Option<&str>) -> Result<scaffold::Template> {
    if bin && lib {
        bail!("choose `--lib` OR `--bin`, not both");
    }
    match template {
        Some("seiva") => Ok(scaffold::Template::Seiva),
        Some(other) => bail!("unknown template `{other}` (available: seiva)"),
        None => Ok(if bin {
            scaffold::Template::Bin
        } else {
            scaffold::Template::Lib
        }),
    }
}

fn cmd_new(
    name: &str,
    lib: bool,
    bin: bool,
    template: Option<&str>,
    dry_run: bool,
) -> Result<ExitCode> {
    let tpl = resolve_template(lib, bin, template)?;
    let dir = Utf8PathBuf::from(name);
    if dir.exists()
        && std::fs::read_dir(&dir)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false)
    {
        bail!("`{name}` already exists and is not empty — refusing to overwrite existing work");
    }
    let plan = scaffold::plan_scaffold_template(&dir, scaffold::crate_base_name(name), tpl);
    apply_scaffold(&dir, plan, dry_run)
}

fn cmd_init(lib: bool, bin: bool, template: Option<&str>, dry_run: bool) -> Result<ExitCode> {
    let tpl = resolve_template(lib, bin, template)?;
    let dir = Utf8PathBuf::from(".");
    if dir.join("Cargo.toml").exists() {
        bail!("a Cargo.toml already exists here — `init` does not overwrite");
    }
    let name = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "projecto".into());
    let plan = scaffold::plan_scaffold_template(&dir, scaffold::crate_base_name(&name), tpl);
    apply_scaffold(&dir, plan, dry_run)
}

fn apply_scaffold(root: &Utf8Path, plan: Plan, dry_run: bool) -> Result<ExitCode> {
    print_plan(&plan);
    if dry_run {
        eprintln!("(--dry-run: nothing written)");
        return Ok(ExitCode::SUCCESS);
    }
    if plan.is_noop() {
        return Ok(ExitCode::SUCCESS);
    }
    // Garantir directórios e montar as escritas transaccionais.
    let mut writes = Vec::new();
    for change in &plan.changes {
        if let Change::CreateFile { path, contents } = change {
            let p = Utf8PathBuf::from(path);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create directory {parent}"))?;
            }
            writes.push(FileWrite {
                path: p,
                new_contents: contents.clone(),
            });
        }
    }
    apply_transaction(root, &writes).context("apply scaffold")?;
    eprintln!("\u{2713} {} file(s) created in {root}", writes.len());
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------------------
// add / remove
// ---------------------------------------------------------------------------

struct AddArgs {
    spec: String,
    features: Vec<String>,
    no_default_features: bool,
    optional: bool,
    dev: bool,
    build: bool,
    git: Option<String>,
    rev: Option<String>,
    tag: Option<String>,
    branch: Option<String>,
    path: Option<String>,
    registry: Option<String>,
    manifest_path: Option<Utf8PathBuf>,
    offline: bool,
    plan: bool,
    dry_run: bool,
}

fn cmd_add(a: AddArgs) -> Result<ExitCode> {
    let (name, version_req) = DepSpec::parse_name_at_version(&a.spec)?;

    // Defesa contra typosquatting de extensões/known crates: sugere, não troca.
    if let Some((suggestion, dist)) = suspect::suspect_typo(&name) {
        bail!(
            "suspicious name `{name}`: did you mean `{suggestion}`? (distance {dist})\n\
             Refusing to silently install a look-alike package — rerun with the exact name if you really mean `{name}`."
        );
    }

    let kind = match (a.dev, a.build) {
        (true, false) => DepKind::Dev,
        (false, true) => DepKind::Build,
        (false, false) => DepKind::Normal,
        (true, true) => bail!("`--dev` and `--build` are mutually exclusive"),
    };

    // Resolver a origem a partir das flags (git > path > registry).
    let source = if let Some(url) = a.git {
        Source::Git {
            url,
            rev: a.rev,
            tag: a.tag,
            branch: a.branch,
        }
    } else if let Some(p) = a.path {
        Source::Path { path: p }
    } else {
        Source::Registry {
            registry: a.registry,
        }
    };

    // Regra de identificação inequívoca: registry sem versão explícita é
    // adiado para o inc.2 (resolver "latest" tem de passar pelo gate ANTES de
    // aterrar — resolver-e-aterrar sem avaliação violaria o contrato §5).
    if matches!(source, Source::Registry { .. }) && version_req.is_none() {
        bail!(
            "registry dependency `{name}` without a version: use `{name}@<req>` or `--version` \
             (automatic \"latest\" resolution must pass the security gate before landing)"
        );
    }

    let mut spec = DepSpec {
        name: name.clone(),
        version_req,
        source,
        features: a.features,
        default_features: !a.no_default_features,
        optional: a.optional,
        kind,
        target: None,
        rename: None,
    };
    spec.normalize();

    let (root, manifest_path) = locate_manifest(a.manifest_path.as_deref())?;
    recover(&root).ok(); // completar journal pendente antes de mutar
    let mut manifest = Manifest::load(&manifest_path)?;

    // `--plan`: mostrar o IMPACTO sem aterrar (o que estou a introduzir?).
    if a.plan {
        let mut probe = Manifest::load(&manifest_path)?;
        probe.upsert(&spec);
        let mutated = probe.to_toml_string();
        return plancmd::impact(&root, &manifest_path, &mutated, &spec, a.offline);
    }

    let change = manifest.plan_add(&spec)?;
    let mut plan = Plan::new();
    if let Some(c) = change {
        plan.push(c);
    }
    print_plan(&plan);

    if a.dry_run {
        eprintln!("(--dry-run: nothing written)");
        return Ok(ExitCode::SUCCESS);
    }
    if plan.is_noop() {
        return Ok(ExitCode::SUCCESS); // idempotente: nenhuma escrita
    }

    // Mutar em memória (ainda NÃO escrito em disco) e resolver o grafo
    // candidato num staging isolado, SEM compilar nem correr build.rs.
    manifest.upsert(&spec);
    let mutated = manifest.to_toml_string();

    let commit = |m: &Manifest| -> Result<()> {
        commit_manifest(&root, m).context("write Cargo.toml")?;
        eprintln!(
            "\u{2713} `{}` configured in {manifest_path}",
            spec.table_entry_key()
        );
        Ok(())
    };

    match stage::resolve_candidate(&root, &mutated, a.offline) {
        stage::Staging::Resolved { root: staging, .. } => {
            let policy = Policy::from_env();
            // Cache sob o projecto real; auditoria sobre o lockfile do staging.
            let outcome = run_gate(&root, &staging, &policy, security::now_unix())
                .context("run the security gate (is cargo-audit installed?)")?;
            security::report_verdict(
                &outcome.verdict,
                &staging,
                outcome.from_cache,
                outcome.deny_covered,
            );
            match outcome.verdict.decision {
                Decision::Approved => {
                    commit(&manifest)?;
                    if let Some(spec) = dx_seiva::known(&name) {
                        return seivacmd::integrate(&root, &manifest_path, &spec, true);
                    }
                    Ok(ExitCode::SUCCESS)
                }
                Decision::Blocked => {
                    eprintln!(
                        "\u{2717} `{name}` did NOT land: candidate graph blocked. Manifest unchanged."
                    );
                    Ok(ExitCode::from(outcome.verdict.decision.exit_code()))
                }
                Decision::Pending => {
                    eprintln!(
                        "\u{23f8} `{name}` did NOT land: advisory base invalid/expired. Renew the base (`dx doctor`) and retry; the download may have happened, USE is not authorized."
                    );
                    Ok(ExitCode::from(outcome.verdict.decision.exit_code()))
                }
            }
        }
        stage::Staging::NotApplicable { reason } => {
            // Honesto: não finjo gate. A edição de texto aterra (não compila
            // nada), mas o estado é NÃO-VERIFICADO e é preciso `dx audit`.
            commit(&manifest)?;
            eprintln!(
                "\u{26a0} pre-landing gate NOT applied ({reason}). The dependency was written, but the graph was not evaluated in staging. Run `dx audit` before building."
            );
            if let Some(spec) = dx_seiva::known(&name) {
                return seivacmd::integrate(&root, &manifest_path, &spec, false);
            }
            Ok(ExitCode::from(6))
        }
        stage::Staging::ResolveFailed { reason } => {
            eprintln!(
                "error: could not resolve the candidate graph; manifest unchanged.\n  {reason}"
            );
            Ok(ExitCode::FAILURE)
        }
    }
}

fn cmd_remove(
    name: &str,
    dev: bool,
    build: bool,
    manifest_path: Option<Utf8PathBuf>,
    dry_run: bool,
) -> Result<ExitCode> {
    let kind = match (dev, build) {
        (true, false) => DepKind::Dev,
        (false, true) => DepKind::Build,
        (false, false) => DepKind::Normal,
        (true, true) => bail!("`--dev` and `--build` are mutually exclusive"),
    };
    let (root, manifest_path) = locate_manifest(manifest_path.as_deref())?;
    recover(&root).ok();
    let mut manifest = Manifest::load(&manifest_path)?;

    let mut plan = Plan::new();
    if let Some(c) = manifest.plan_remove(name, kind, None) {
        plan.push(c);
    }
    print_plan(&plan);
    if dry_run {
        eprintln!("(--dry-run: nothing written)");
        return Ok(ExitCode::SUCCESS);
    }
    if plan.is_noop() {
        return Ok(ExitCode::SUCCESS); // já não existe
    }
    manifest.remove(name, kind, None);
    commit_manifest(&root, &manifest).context("escrever Cargo.toml")?;
    eprintln!("\u{2713} `{name}` removed from {manifest_path}");
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------------------
// utilitários
// ---------------------------------------------------------------------------

/// Localiza o Cargo.toml (explícito ou subindo a partir do CWD) e a raiz
/// (directório que o contém) para o `.dx/`.
fn locate_manifest(explicit: Option<&Utf8Path>) -> Result<(Utf8PathBuf, Utf8PathBuf)> {
    if let Some(p) = explicit {
        let root = p
            .parent()
            .map(|x| x.to_owned())
            .unwrap_or_else(|| Utf8PathBuf::from("."));
        return Ok((root, p.to_owned()));
    }
    let cwd = std::env::current_dir().context("CWD")?;
    let mut dir = Utf8PathBuf::from_path_buf(cwd).map_err(|_| anyhow::anyhow!("non-UTF8 CWD"))?;
    loop {
        let candidate = dir.join("Cargo.toml");
        if candidate.exists() {
            return Ok((dir.clone(), candidate));
        }
        if !dir.pop() {
            bail!("no Cargo.toml found from the current directory");
        }
    }
}

fn print_plan(plan: &Plan) {
    if plan.is_noop() {
        eprintln!("Plan: (no changes — already converged)");
    } else {
        eprintln!("Plan ({} change(s)):", plan.len());
        eprintln!("{plan}");
    }
}

/// Delega no Cargo com argv estruturado (sem shell), herdando stdio e código
/// de saída. Preserva sinais do processo-filho via o exit status.
fn cargo_passthrough(args: &[String]) -> Result<ExitCode> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .args(args)
        .status()
        .context("could not invoke cargo")?;
    match status.code() {
        Some(code) => Ok(ExitCode::from(code as u8)),
        None => Ok(ExitCode::from(130)), // terminado por sinal
    }
}
