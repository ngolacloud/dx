//! Veredicto e cache de decisão.
//!
//! Regras (contrato §5):
//! - Achado que bloqueia ⇒ `Blocked` (mesmo que a base esteja fresca).
//! - Grafo limpo mas base **expirada** ⇒ `Pending` (uso não autorizado; o
//!   download pode ter terminado, a autorização não).
//! - Grafo limpo e base válida ⇒ `Approved`.
//!
//! A chave do cache inclui lockfile + política + snapshot da base + versão do
//! scanner. Um aviso novo (snapshot muda) invalida veredictos antigos mesmo
//! com downloads intactos.

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::audit::Finding;
use crate::db::DbStatus;
use crate::policy::{Action, Policy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Decision {
    Approved,
    Blocked,
    Pending,
}

impl Decision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Decision::Approved => "approved",
            Decision::Blocked => "blocked",
            Decision::Pending => "pending",
        }
    }
    /// Código de saída associado (0 aprovado; !=0 caso contrário).
    pub fn exit_code(&self) -> u8 {
        match self {
            Decision::Approved => 0,
            Decision::Blocked => 4,
            Decision::Pending => 5,
        }
    }
}

/// Chave do cache de decisão.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheKey {
    pub lockfile_sha: String,
    pub policy_fp: String,
    pub db_snapshot: String,
    pub scanner_version: String,
}

impl CacheKey {
    pub fn digest(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.lockfile_sha.as_bytes());
        h.update(b"|");
        h.update(self.policy_fp.as_bytes());
        h.update(b"|");
        h.update(self.db_snapshot.as_bytes());
        h.update(b"|");
        h.update(self.scanner_version.as_bytes());
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    pub decision: Decision,
    pub findings: Vec<Finding>,
    /// Índices (em `findings`) dos achados que bloqueiam.
    pub blocking: Vec<usize>,
    pub db_status: DbStatus,
    pub key: CacheKey,
}

impl Verdict {
    pub fn is_authorized(&self) -> bool {
        self.decision == Decision::Approved
    }
}

/// Avalia os achados contra a política e o estado da base.
pub fn evaluate(
    findings: Vec<Finding>,
    policy: &Policy,
    db_status: DbStatus,
    key: CacheKey,
) -> Verdict {
    let mut blocking = Vec::new();
    for (i, f) in findings.iter().enumerate() {
        if policy.action_for(f.category) == Action::Block {
            blocking.push(i);
        }
    }
    let decision = if !blocking.is_empty() {
        Decision::Blocked
    } else if !db_status.valid {
        Decision::Pending
    } else {
        Decision::Approved
    };
    Verdict {
        decision,
        findings,
        blocking,
        db_status,
        key,
    }
}

/// Hash sha256 (hex) de um conteúdo de ficheiro (ex.: Cargo.lock).
pub fn sha256_file(path: &Utf8Path) -> std::io::Result<String> {
    let data = std::fs::read(path)?;
    let mut h = Sha256::new();
    h.update(&data);
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn cache_dir(root: &Utf8Path) -> Utf8PathBuf {
    root.join(".dx/security")
}

fn cache_path(root: &Utf8Path, key: &CacheKey) -> Utf8PathBuf {
    cache_dir(root).join(format!("decision-{}.json", key.digest()))
}

/// Guarda um veredicto no cache (chave exacta).
pub fn save(root: &Utf8Path, verdict: &Verdict) -> std::io::Result<()> {
    std::fs::create_dir_all(cache_dir(root))?;
    let path = cache_path(root, &verdict.key);
    let data = serde_json::to_string_pretty(verdict).map_err(std::io::Error::other)?;
    std::fs::write(path, data)
}

/// Carrega um veredicto do cache **apenas** se a chave bater exactamente.
/// Qualquer mudança (lockfile, política, snapshot da base, scanner) resulta em
/// `None` — revalidação obrigatória.
pub fn load(root: &Utf8Path, key: &CacheKey) -> Option<Verdict> {
    let path = cache_path(root, key);
    let data = std::fs::read_to_string(path).ok()?;
    let verdict: Verdict = serde_json::from_str(&data).ok()?;
    if &verdict.key == key {
        Some(verdict)
    } else {
        None
    }
}

/// Estado de confiança barato, para o caminho quente (ADR-0021).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustState {
    /// Existe um veredicto `approved` para este exacto lockfile. Prosseguir em silêncio.
    Approved,
    /// Há veredictos, mas o lockfile mudou (ou o último não foi aprovado).
    Stale,
    /// O projecto nunca estabeleceu uma baseline de confiança.
    Unknown,
    /// Nada resolvido ainda (sem Cargo.lock).
    NoLockfile,
}

/// Consulta de confiança **sem spawnar nenhum processo** (ADR-0021).
///
/// Responde «este lockfile já foi aprovado?» lendo apenas o `Cargo.lock` e os
/// veredictos em `.dx/security/decision-*.json`. **Nunca** corre
/// `cargo-audit`/`cargo-deny`/`rustc` (construir a `CacheKey` completa exigiria
/// `cargo audit --version`, um spawn — evitado de propósito).
///
/// Casa pelo `lockfile_sha` (a parte que muda quando as dependências mudam). É
/// um **sinal barato**, não o gate: ignora drift de política/base de advisories
/// — se estes mudaram, o `dx audit`/`dx add` (scanner real) reavaliam. Nunca
/// aprova nada; aprovar é sempre do gate real.
pub fn fast_trust(root: &Utf8Path, manifest_dir: &Utf8Path) -> TrustState {
    let lockfile = manifest_dir.join("Cargo.lock");
    let current = match sha256_file(&lockfile) {
        Ok(sha) => sha,
        Err(_) => return TrustState::NoLockfile,
    };
    let dir = cache_dir(root);
    let entries = match std::fs::read_dir(dir.as_std_path()) {
        Ok(e) => e,
        Err(_) => return TrustState::Unknown,
    };
    let mut saw_any = false;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let Ok(data) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(verdict) = serde_json::from_str::<Verdict>(&data) else {
            continue;
        };
        saw_any = true;
        if verdict.decision == Decision::Approved && verdict.key.lockfile_sha == current {
            return TrustState::Approved;
        }
    }
    if saw_any {
        TrustState::Stale
    } else {
        TrustState::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbStatus;
    use crate::policy::Category;

    fn mkfinding(cat: Category) -> Finding {
        Finding {
            category: cat,
            package: "x".into(),
            version: "1".into(),
            advisory_id: Some("RUSTSEC-0000-0000".into()),
            title: "t".into(),
            url: None,
            dependency_path: None,
        }
    }
    fn valid_db() -> DbStatus {
        DbStatus {
            path: "/x".into(),
            last_commit_unix: Some(1000),
            age_secs: Some(10),
            max_age_secs: 86400,
            valid: true,
            detail: "ok".into(),
        }
    }
    fn expired_db() -> DbStatus {
        DbStatus {
            valid: false,
            detail: "expirada".into(),
            ..valid_db()
        }
    }
    fn key() -> CacheKey {
        CacheKey {
            lockfile_sha: "l".into(),
            policy_fp: "p".into(),
            db_snapshot: "s".into(),
            scanner_version: "v".into(),
        }
    }

    #[test]
    fn vulnerability_blocks_even_with_fresh_db() {
        let v = evaluate(
            vec![mkfinding(Category::Vulnerability)],
            &Policy::default(),
            valid_db(),
            key(),
        );
        assert_eq!(v.decision, Decision::Blocked);
        assert_eq!(v.decision.exit_code(), 4);
    }

    #[test]
    fn clean_but_expired_is_pending_not_approved() {
        let v = evaluate(vec![], &Policy::default(), expired_db(), key());
        assert_eq!(v.decision, Decision::Pending);
        assert!(!v.is_authorized());
    }

    #[test]
    fn clean_and_fresh_is_approved() {
        let v = evaluate(vec![], &Policy::default(), valid_db(), key());
        assert_eq!(v.decision, Decision::Approved);
    }

    #[test]
    fn yanked_warns_does_not_block() {
        let v = evaluate(
            vec![mkfinding(Category::Yanked)],
            &Policy::default(),
            valid_db(),
            key(),
        );
        assert_eq!(v.decision, Decision::Approved);
        assert!(v.blocking.is_empty());
    }

    #[test]
    fn cache_roundtrip_and_key_sensitivity() {
        let tmp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(tmp.path()).unwrap();
        let v = evaluate(vec![], &Policy::default(), valid_db(), key());
        save(root, &v).unwrap();
        assert!(load(root, &key()).is_some());
        // Snapshot da base diferente ⇒ cache miss (aviso novo invalida).
        let mut k2 = key();
        k2.db_snapshot = "outro".into();
        assert!(load(root, &k2).is_none());
    }

    /// Escreve um Cargo.lock e devolve o seu sha256 (a parte que `fast_trust`
    /// casa). O `manifest_dir` é o `root` neste cenário.
    fn write_lock(root: &Utf8Path, contents: &str) -> String {
        std::fs::write(root.join("Cargo.lock"), contents).unwrap();
        sha256_file(&root.join("Cargo.lock")).unwrap()
    }

    fn approved_verdict_for(lock_sha: &str) -> Verdict {
        let mut k = key();
        k.lockfile_sha = lock_sha.to_string();
        evaluate(vec![], &Policy::default(), valid_db(), k)
    }

    #[test]
    fn fast_trust_no_lockfile_is_nolockfile() {
        let tmp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(tmp.path()).unwrap();
        assert_eq!(fast_trust(root, root), TrustState::NoLockfile);
    }

    #[test]
    fn fast_trust_lockfile_but_no_cache_is_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(tmp.path()).unwrap();
        write_lock(root, "v1");
        assert_eq!(fast_trust(root, root), TrustState::Unknown);
    }

    #[test]
    fn fast_trust_matching_approved_is_approved() {
        let tmp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(tmp.path()).unwrap();
        let sha = write_lock(root, "v1");
        save(root, &approved_verdict_for(&sha)).unwrap();
        assert_eq!(fast_trust(root, root), TrustState::Approved);
    }

    #[test]
    fn fast_trust_changed_lockfile_is_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(tmp.path()).unwrap();
        let sha = write_lock(root, "v1");
        save(root, &approved_verdict_for(&sha)).unwrap();
        // O lockfile muda ⇒ o veredicto aprovado deixa de casar.
        write_lock(root, "v2-different");
        assert_eq!(fast_trust(root, root), TrustState::Stale);
    }

    #[test]
    fn fast_trust_blocked_verdict_same_lock_is_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(tmp.path()).unwrap();
        let sha = write_lock(root, "v1");
        // Veredicto existente mas BLOCKED ⇒ não é confiança, é Stale.
        let mut k = key();
        k.lockfile_sha = sha;
        let blocked = evaluate(
            vec![mkfinding(Category::Vulnerability)],
            &Policy::default(),
            valid_db(),
            k,
        );
        assert_eq!(blocked.decision, Decision::Blocked);
        save(root, &blocked).unwrap();
        assert_eq!(fast_trust(root, root), TrustState::Stale);
    }
}
