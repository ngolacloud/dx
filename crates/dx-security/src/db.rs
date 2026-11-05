//! Validade da base de advisories (RustSec advisory-db).
//!
//! A frescura é determinada pelo DX a partir do timestamp do commit HEAD do
//! clone local da advisory-db — não dependemos de o cargo-audit a reportar
//! (nesta versão vem `null`). Base expirada ⇒ não se autoriza (contrato §5),
//! embora os achados ainda sejam mostrados a título informativo.

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use std::process::Command;

/// Estado de frescura da base num dado instante.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbStatus {
    pub path: Utf8PathBuf,
    /// Timestamp (unix) do commit HEAD, se foi possível lê-lo.
    pub last_commit_unix: Option<u64>,
    /// Idade em segundos face ao `now` fornecido.
    pub age_secs: Option<u64>,
    /// Validade máxima aplicada (segundos).
    pub max_age_secs: u64,
    /// `true` se dentro da validade.
    pub valid: bool,
    /// Descrição do porquê de `valid`/inválido.
    pub detail: String,
}

/// Caminho por omissão da advisory-db (o mesmo que o cargo-audit usa).
pub fn default_db_path() -> Utf8PathBuf {
    let home = std::env::var("CARGO_HOME")
        .ok()
        .map(Utf8PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| Utf8PathBuf::from(h).join(".cargo"))
        })
        .unwrap_or_else(|| Utf8PathBuf::from(".cargo"));
    home.join("advisory-db")
}

/// Lê o timestamp do commit HEAD de um repositório git. Memoizado por caminho
/// no processo (o HEAD é invariante durante uma execução; evita spawns de git
/// repetidos a cada chamada do gate — ver dx bench / ADR-0014).
fn git_head_commit_unix(repo: &Utf8Path) -> Option<u64> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Option<u64>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = cache.lock().unwrap().get(repo.as_str()) {
        return *v;
    }
    let out = Command::new("git")
        .args(["-C", repo.as_str(), "log", "-1", "--format=%ct"])
        .output()
        .ok();
    let value = out.filter(|o| o.status.success()).and_then(|o| {
        String::from_utf8_lossy(&o.stdout)
            .trim()
            .parse::<u64>()
            .ok()
    });
    cache.lock().unwrap().insert(repo.to_string(), value);
    value
}

/// Avalia a frescura da base. `now_unix` é injectado para testabilidade
/// (a CLI passa o instante real).
pub fn evaluate(db_path: &Utf8Path, now_unix: u64, max_age_secs: u64) -> DbStatus {
    if !db_path.exists() {
        return DbStatus {
            path: db_path.to_owned(),
            last_commit_unix: None,
            age_secs: None,
            max_age_secs,
            valid: false,
            detail: format!("base ausente em {db_path} — correr um refresh antes de usar"),
        };
    }
    match git_head_commit_unix(db_path) {
        Some(ts) => {
            let age = now_unix.saturating_sub(ts);
            let valid = age <= max_age_secs;
            let detail = if valid {
                format!(
                    "base com {}h (dentro de {}h)",
                    age / 3600,
                    max_age_secs / 3600
                )
            } else {
                format!(
                    "base com {}h — EXPIRADA (máx {}h); renovar antes de autorizar",
                    age / 3600,
                    max_age_secs / 3600
                )
            };
            DbStatus {
                path: db_path.to_owned(),
                last_commit_unix: Some(ts),
                age_secs: Some(age),
                max_age_secs,
                valid,
                detail,
            }
        }
        None => DbStatus {
            path: db_path.to_owned(),
            last_commit_unix: None,
            age_secs: None,
            max_age_secs,
            valid: false,
            detail: format!(
                "não consegui ler o commit HEAD de {db_path} (git?) — trato como inválida"
            ),
        },
    }
}

/// Identificador curto do snapshot da base (para a chave do cache de decisão).
/// Memoizado por caminho no processo (invariante durante a execução).
pub fn snapshot_id(db_path: &Utf8Path) -> String {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = cache.lock().unwrap().get(db_path.as_str()) {
        return v.clone();
    }
    let out = Command::new("git")
        .args(["-C", db_path.as_str(), "rev-parse", "HEAD"])
        .output();
    let id = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => "unknown".into(),
    };
    cache
        .lock()
        .unwrap()
        .insert(db_path.to_string(), id.clone());
    id
}
