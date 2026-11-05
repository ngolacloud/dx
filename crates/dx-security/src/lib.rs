//! `dx-security` — gate de segurança da cadeia de dependências.
//!
//! Fundação do incremento 2 (ADR-0005). Reutiliza o `cargo-audit` (RustSec)
//! como scanner, gere a **validade da base** de advisories e emite um
//! **veredicto** (approved/blocked/pending) com cache de decisão. Separa
//! claramente *descarregar* de *autorizar*.
//!
//! Limites honestos desta fundação (ver `docs/progress.md`):
//! - A resolução em staging isolado **sem** executar build.rs ainda não está
//!   ligada ao `add` (o `audit` opera sobre o Cargo.lock existente do projecto).
//! - O isolamento de build de dependências (rede desligada, FS mínimo) é
//!   previsto e ainda não implementado.

pub mod audit;
pub mod db;
pub mod policy;
pub mod verdict;

pub use audit::{inverse_dependency_path, Finding};
pub use db::{default_db_path, DbStatus};
pub use policy::{Action, Category, Policy};
pub use verdict::{CacheKey, Decision, Verdict};

use camino::Utf8Path;

/// Erros do gate.
#[derive(Debug, thiserror::Error)]
pub enum GateError {
    #[error(transparent)]
    Audit(#[from] audit::AuditError),
    #[error("erro de I/O no gate: {0}")]
    Io(#[from] std::io::Error),
}

/// Resultado de correr o gate, distinguindo se veio do cache.
pub struct GateOutcome {
    pub verdict: Verdict,
    pub from_cache: bool,
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

    // Chave do cache: lockfile + política + snapshot da base + versão do scanner.
    let lockfile = manifest_dir.join("Cargo.lock");
    let lockfile_sha = verdict::sha256_file(&lockfile).unwrap_or_else(|_| "no-lockfile".into());
    let key = CacheKey {
        lockfile_sha,
        policy_fp: policy.fingerprint(),
        db_snapshot: db::snapshot_id(&db_path),
        scanner_version: audit::scanner_version(),
    };

    // Reutilizar veredicto só com chave exacta (aviso novo ⇒ snapshot muda ⇒ miss).
    if let Some(cached) = verdict::load(root, &key) {
        return Ok(GateOutcome {
            verdict: cached,
            from_cache: true,
        });
    }

    let findings = audit::run(manifest_dir)?;
    let v = verdict::evaluate(findings, policy, db_status, key);
    verdict::save(root, &v)?;
    Ok(GateOutcome {
        verdict: v,
        from_cache: false,
    })
}
