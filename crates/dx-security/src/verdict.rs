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
}
