//! `dx` — CLI fina sobre bibliotecas testáveis (dx-core, dx-manifest).
//!
//! Incremento 1: contrato Cargo e mutações idempotentes (`new`/`init`/`add`/
//! `remove`), com `--dry-run`, plano antes de alterar, journal e lock. Os
//! comandos de compilação/teste delegam no Cargo com argv estruturado (sem
//! shell). O gate de segurança, `update`/`fetch`, `audit`, `doctor` e
//! `release` chegam em incrementos seguintes e estão marcados como tal.

mod scaffold;
mod security;
mod stage;
mod suspect;

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
    about = "DX — experiência de ferramentas Rust inspirada no UV (executável: dx)",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Criar um projecto novo (`--lib` ou `--bin`).
    New {
        name: String,
        #[arg(long)]
        lib: bool,
        #[arg(long)]
        bin: bool,
        #[command(flatten)]
        dry: DryRun,
    },
    /// Inicializar um projecto no directório actual.
    Init {
        #[arg(long)]
        lib: bool,
        #[arg(long)]
        bin: bool,
        #[command(flatten)]
        dry: DryRun,
    },
    /// Adicionar uma dependência (idempotente).
    Add {
        /// `nome` ou `nome@requisito`.
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
        /// Resolver o grafo candidato sem rede (usa só o cache do Cargo).
        #[arg(long)]
        offline: bool,
        #[command(flatten)]
        dry: DryRun,
    },
    /// Remover uma dependência (idempotente).
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
    /// Formatar (delega em `cargo fmt`).
    Fmt {
        #[arg(long)]
        check: bool,
    },
    /// Lint (delega em `cargo clippy`).
    Lint,
    /// Verificar sem compilar binário final (delega em `cargo check`).
    Check,
    /// Compilar (delega em `cargo build`).
    Build {
        #[arg(long)]
        release: bool,
        /// Compilar com isolamento real (rede desligada, ambiente sem
        /// segredos, limites de recursos). Usa `--offline` (deps já obtidas).
        #[arg(long)]
        isolated: bool,
    },
    /// Executar (delega em `cargo run`).
    Run {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Testar (delega em `cargo test`; `nextest` previsto no inc.3).
    Test {
        #[arg(long)]
        suite: Option<String>,
    },
    /// [inc.2] Auditoria de segurança do grafo.
    Audit,
    /// [inc.2] Diagnóstico do ambiente.
    Doctor,
    /// [inc.2] Actualizar dependências (consulta universo novo).
    Update { name: Option<String> },
    /// Verificar a integridade do grafo bloqueado (sem compilar).
    Fetch {
        /// Exigir um Cargo.lock existente (falha se não houver).
        #[arg(long)]
        locked: bool,
        /// Além do checksum, desempacotar cada `.crate` com salvaguardas
        /// (traversal/symlink/zip-bomb) para um temporário e descartar.
        #[arg(long)]
        extract_check: bool,
    },
    /// [inc.4] Preparar release.
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
    /// Mostrar o plano exacto sem alterar ficheiros.
    #[arg(long)]
    dry_run: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("erro: {e:#}");
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
            dry,
        } => cmd_new(&name, lib, bin, dry.dry_run),
        Commands::Init { lib, bin, dry } => cmd_init(lib, bin, dry.dry_run),
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
        Commands::Build { release, isolated } => {
            if isolated {
                return security::cmd_build_isolated(release);
            }
            let mut a = vec!["build".to_string()];
            if release {
                a.push("--release".into());
            }
            cargo_passthrough(&a)
        }
        Commands::Run { args } => {
            let mut a = vec!["run".to_string()];
            if !args.is_empty() {
                a.push("--".into());
                a.extend(args);
            }
            cargo_passthrough(&a)
        }
        Commands::Test { suite } => {
            // Mapeamento de suites honesto: sem configuração versionada ainda,
            // só `unit`/`doc` têm significado directo em Cargo. Ver inc.3.
            match suite.as_deref() {
                None | Some("unit") => {
                    cargo_passthrough(&["test".into(), "--lib".into(), "--bins".into()])
                }
                Some("doc") => cargo_passthrough(&["test".into(), "--doc".into()]),
                Some("integration") => {
                    cargo_passthrough(&["test".into(), "--test".into(), "*".into()])
                }
                Some(other) => {
                    bail!("suite `{other}` exige runner configurado em dx.toml (planeado no inc.3); sem config, não a classifico por nome")
                }
            }
        }
        Commands::Audit => {
            let (root, manifest_path) = locate_manifest(None)?;
            let _ = manifest_path;
            security::cmd_audit(&root)
        }
        Commands::Doctor => security::cmd_doctor(),
        Commands::Update { .. } => not_yet(
            "update",
            2,
            "actualização com reavaliação de segurança do universo novo",
        ),
        Commands::Fetch {
            locked,
            extract_check,
        } => {
            let (root, manifest_path) = locate_manifest(None)?;
            let _ = manifest_path;
            security::cmd_fetch(&root, locked, extract_check)
        }
        Commands::Release { .. } => not_yet(
            "release prepare",
            4,
            "validações + binários/SBOM/proveniência em ambiente limpo",
        ),
    }
}

/// Mensagem honesta para comandos ainda não implementados.
fn not_yet(name: &str, increment: u8, what: &str) -> Result<ExitCode> {
    eprintln!(
        "`dx {name}` ainda não está implementado (previsto no incremento {increment}): {what}."
    );
    eprintln!("Entretanto, invocar o Cargo directamente funciona mas CONTORNA o gate DX (ver docs/security.md).");
    Ok(ExitCode::from(3))
}

// ---------------------------------------------------------------------------
// new / init
// ---------------------------------------------------------------------------

fn cmd_new(name: &str, lib: bool, bin: bool, dry_run: bool) -> Result<ExitCode> {
    if bin && lib {
        bail!("escolhe `--lib` OU `--bin`, não ambos");
    }
    let is_lib = lib && !bin;
    let dir = Utf8PathBuf::from(name);
    if dir.exists()
        && std::fs::read_dir(&dir)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false)
    {
        bail!("`{name}` já existe e não está vazio — não sobrescrevo trabalho existente");
    }
    let plan = scaffold::plan_scaffold(&dir, scaffold::crate_base_name(name), is_lib);
    apply_scaffold(&dir, plan, dry_run)
}

fn cmd_init(lib: bool, bin: bool, dry_run: bool) -> Result<ExitCode> {
    if bin && lib {
        bail!("escolhe `--lib` OU `--bin`, não ambos");
    }
    let is_lib = lib && !bin;
    let dir = Utf8PathBuf::from(".");
    if dir.join("Cargo.toml").exists() {
        bail!("já existe um Cargo.toml aqui — `init` não sobrescreve");
    }
    let name = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "projecto".into());
    let plan = scaffold::plan_scaffold(&dir, scaffold::crate_base_name(&name), is_lib);
    apply_scaffold(&dir, plan, dry_run)
}

fn apply_scaffold(root: &Utf8Path, plan: Plan, dry_run: bool) -> Result<ExitCode> {
    print_plan(&plan);
    if dry_run {
        eprintln!("(--dry-run: nada escrito)");
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
                    .with_context(|| format!("criar directório {parent}"))?;
            }
            writes.push(FileWrite {
                path: p,
                new_contents: contents.clone(),
            });
        }
    }
    apply_transaction(root, &writes).context("aplicar scaffold")?;
    eprintln!("✓ {} ficheiro(s) criado(s) em {root}", writes.len());
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
    dry_run: bool,
}

fn cmd_add(a: AddArgs) -> Result<ExitCode> {
    let (name, version_req) = DepSpec::parse_name_at_version(&a.spec)?;

    // Defesa contra typosquatting de extensões/known crates: sugere, não troca.
    if let Some((suggestion, dist)) = suspect::suspect_typo(&name) {
        bail!(
            "nome suspeito `{name}`: quis dizer `{suggestion}`? (distância {dist})\n\
             Recuso instalar silenciosamente um package parecido — reexecuta com o nome exacto se for mesmo `{name}`."
        );
    }

    let kind = match (a.dev, a.build) {
        (true, false) => DepKind::Dev,
        (false, true) => DepKind::Build,
        (false, false) => DepKind::Normal,
        (true, true) => bail!("`--dev` e `--build` são mutuamente exclusivos"),
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
            "dependência de registry `{name}` sem versão: usa `{name}@<req>` ou `--version` \
             (a resolução automática de \"latest\" chega no inc.2, porque tem de passar pelo gate de segurança antes de aterrar)"
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

    let change = manifest.plan_add(&spec)?;
    let mut plan = Plan::new();
    if let Some(c) = change {
        plan.push(c);
    }
    print_plan(&plan);

    if a.dry_run {
        eprintln!("(--dry-run: nada escrito)");
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
        commit_manifest(&root, m).context("escrever Cargo.toml")?;
        eprintln!(
            "✓ `{}` configurado em {manifest_path}",
            spec.table_entry_key()
        );
        Ok(())
    };

    match stage::resolve_candidate(&root, &mutated, a.offline) {
        stage::Staging::Resolved { root: staging, .. } => {
            let policy = Policy::from_env();
            // Cache sob o projecto real; auditoria sobre o lockfile do staging.
            let outcome = run_gate(&root, &staging, &policy, security::now_unix())
                .context("correr o gate de segurança (cargo-audit instalado?)")?;
            security::report_verdict(
                &outcome.verdict,
                &staging,
                outcome.from_cache,
                outcome.deny_covered,
            );
            match outcome.verdict.decision {
                Decision::Approved => {
                    commit(&manifest)?;
                    Ok(ExitCode::SUCCESS)
                }
                Decision::Blocked => {
                    eprintln!(
                        "✗ `{name}` NÃO aterrou: grafo candidato bloqueado. Manifesto inalterado."
                    );
                    Ok(ExitCode::from(outcome.verdict.decision.exit_code()))
                }
                Decision::Pending => {
                    eprintln!(
                        "⏸ `{name}` NÃO aterrou: base de advisories inválida/expirada. Renova a base (`dx doctor`) e repete; o download pode ter ocorrido, o USO não está autorizado."
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
                "⚠ gate pré-aterragem NÃO aplicado ({reason}). A dependência foi escrita, mas o grafo não foi avaliado em staging. Corre `dx audit` antes de compilar."
            );
            Ok(ExitCode::from(6))
        }
        stage::Staging::ResolveFailed { reason } => {
            eprintln!(
                "erro: não consegui resolver o grafo candidato; manifesto inalterado.\n  {reason}"
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
        (true, true) => bail!("`--dev` e `--build` são mutuamente exclusivos"),
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
        eprintln!("(--dry-run: nada escrito)");
        return Ok(ExitCode::SUCCESS);
    }
    if plan.is_noop() {
        return Ok(ExitCode::SUCCESS); // já não existe
    }
    manifest.remove(name, kind, None);
    commit_manifest(&root, &manifest).context("escrever Cargo.toml")?;
    eprintln!("✓ `{name}` removido de {manifest_path}");
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
    let mut dir = Utf8PathBuf::from_path_buf(cwd).map_err(|_| anyhow::anyhow!("CWD não-UTF8"))?;
    loop {
        let candidate = dir.join("Cargo.toml");
        if candidate.exists() {
            return Ok((dir.clone(), candidate));
        }
        if !dir.pop() {
            bail!("não encontrei um Cargo.toml a partir do directório actual");
        }
    }
}

fn print_plan(plan: &Plan) {
    if plan.is_noop() {
        eprintln!("Plano: (sem alterações — já convergido)");
    } else {
        eprintln!("Plano ({} alteração(ões)):", plan.len());
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
        .context("não consegui invocar o cargo")?;
    match status.code() {
        Some(code) => Ok(ExitCode::from(code as u8)),
        None => Ok(ExitCode::from(130)), // terminado por sinal
    }
}
