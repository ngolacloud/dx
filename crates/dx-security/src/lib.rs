//! `dx-security` — gate de segurança da cadeia de dependências.
//!
//! Incremento 2 (ADR-0005/0006/0007). Reutiliza o `cargo-audit` (RustSec, para
//! advisories) e o `cargo-deny` (licenças/origens/bans), gere a **validade da
//! base** de advisories e emite um **veredicto** (approved/blocked/pending) com
//! cache de decisão. Separa claramente *descarregar* de *autorizar*. O gate
//! corre no `dx add` sobre um grafo candidato resolvido em staging isolado
//! **sem** executar build.rs/proc-macros (ver dx-cli::stage).
//!
//! Limites honestos (ver `docs/progress.md`):
//! - O isolamento de build de dependências (rede desligada, FS mínimo, limites)
//!   é previsto e ainda não implementado.
//! - A verificação de integridade própria (checksum/extracção segura) ainda não
//!   está implementada.

pub mod audit;
pub mod db;
pub mod deny;
pub mod policy;
pub mod verdict;

pub use audit::{inverse_dependency_path, Finding};
pub use db::{default_db_path, DbStatus};
pub use policy::{Action, Category, Policy};
pub use verdict::{fast_trust, CacheKey, Decision, TrustState, Verdict};

use camino::Utf8Path;

/// Erros do gate.
#[derive(Debug, thiserror::Error)]
pub enum GateError {
    #[error(transparent)]
    Audit(#[from] audit::AuditError),
    #[error("gate I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Resultado de correr o gate, distinguindo se veio do cache.
pub struct GateOutcome {
    pub verdict: Verdict,
    pub from_cache: bool,
    /// `true` se o `cargo-deny` correu (licenças/origens/bans cobertos).
    /// `false` ⇒ essas dimensões ficam "sem cobertura" (honestidade, não falha).
    pub deny_covered: bool,
}

/// Corre o gate de segurança sobre um projecto.
///
/// `root` é a raiz (onde vive `.dx/` e, tipicamente, o Cargo.lock).
/// `now_unix` é injectado para testabilidade (a CLI passa o instante real).
///
/// Com base **válida**: avalia (ou reutiliza veredicto com chave válida).
/// Com base **expirada**: um grafo limpo fica `Pending` (não autorizado); um
/// grafo com bloqueios fica `Blocked`. Nunca converte indisponibilidade em
/// aprovação.
pub fn run_gate(
    root: &Utf8Path,
    manifest_dir: &Utf8Path,
    policy: &Policy,
    now_unix: u64,
) -> Result<GateOutcome, GateError> {
    let db_path = default_db_path();
    let db_status = db::evaluate(&db_path, now_unix, policy.max_db_age_secs);
    let deny_covered = deny::available();

    // Chave do cache: lockfile + política + snapshot da base + versão do scanner.
    // A cobertura do cargo-deny entra na chave para que ligar/desligar o deny
    // invalide veredictos antigos.
    let lockfile = manifest_dir.join("Cargo.lock");
    let lockfile_sha = verdict::sha256_file(&lockfile).unwrap_or_else(|_| "no-lockfile".into());
    let key = CacheKey {
        lockfile_sha,
        policy_fp: policy.fingerprint(),
        db_snapshot: db::snapshot_id(&db_path),
        scanner_version: format!("{}+deny:{}", audit::scanner_version(), deny_covered),
    };

    // Reutilizar veredicto só com chave exacta (aviso novo ⇒ snapshot muda ⇒ miss).
    if let Some(cached) = verdict::load(root, &key) {
        return Ok(GateOutcome {
            verdict: cached,
            from_cache: true,
            deny_covered,
        });
    }

    // cargo-audit (advisories) + cargo-deny (licenças/origens/bans).
    let mut findings = audit::run(manifest_dir)?;
    if deny_covered {
        if let Ok(mut d) = deny::run(manifest_dir) {
            findings.append(&mut d);
        }
    }
    let v = verdict::evaluate(findings, policy, db_status, key);
    verdict::save(root, &v)?;
    Ok(GateOutcome {
        verdict: v,
        from_cache: false,
        deny_covered,
    })
}
