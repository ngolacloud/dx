//! Invocação do `cargo-audit` e normalização dos achados.
//!
//! Reutilizamos o `cargo-audit` (RustSec) em vez de reimplementar o scanner.
//! Corremos com `--no-fetch` para que a frescura da base seja gerida pelo DX
//! (ver `db.rs`) de forma determinística, e com `--json` para parse estável.

use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use std::process::Command;

use crate::policy::Category;

/// Um achado normalizado do scanner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub category: Category,
    pub package: String,
    pub version: String,
    pub advisory_id: Option<String>,
    pub title: String,
    pub url: Option<String>,
    /// Caminho de dependências inverso (quem puxa o package), preenchido a
    /// pedido quando o achado bloqueia.
    pub dependency_path: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    #[error("não consegui invocar `cargo audit` (instalado?): {0}")]
    Spawn(#[from] std::io::Error),
    #[error("`cargo audit` falhou a correr (código {code:?}); stderr:\n{stderr}")]
    Run { code: Option<i32>, stderr: String },
    #[error("saída do `cargo audit` não é JSON válido: {0}")]
    Json(#[from] serde_json::Error),
}

/// Versão do scanner (`cargo audit --version`), para a chave do cache.
pub fn scanner_version() -> String {
    Command::new("cargo")
        .args(["audit", "--version"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "desconhecida".into())
}

/// Corre `cargo audit --no-fetch --json` no directório do projecto e devolve a
/// lista de achados normalizados. **Não** compila nem executa build.rs.
pub fn run(manifest_dir: &Utf8Path) -> Result<Vec<Finding>, AuditError> {
    let out = Command::new("cargo")
        .args(["audit", "--no-fetch", "--json"])
        .current_dir(manifest_dir)
        .output()?;
    // cargo-audit devolve código != 0 quando encontra vulnerabilidades; isso é
    // esperado e não é um erro de execução. Distinguimos pela presença de JSON.
    let stdout = String::from_utf8_lossy(&out.stdout);
    if stdout.trim().is_empty() {
        return Err(AuditError::Run {
            code: out.status.code(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        });
    }
    let v: serde_json::Value = serde_json::from_str(&stdout)?;
    Ok(normalize(&v))
}

/// Converte o JSON do cargo-audit em achados normalizados.
fn normalize(v: &serde_json::Value) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Vulnerabilidades (inclui advisories de malware publicados como advisory,
    // com ou sem CVE — caem aqui e são bloqueadas pela política).
    if let Some(list) = v
        .pointer("/vulnerabilities/list")
        .and_then(|x| x.as_array())
    {
        for item in list {
            findings.push(finding_from(item, classify_advisory(item.get("advisory"))));
        }
    }

    // Avisos, agrupados por tipo (chave == kind).
    if let Some(warnings) = v.get("warnings").and_then(|x| x.as_object()) {
        for (kind, arr) in warnings {
            let cat = kind_to_category(kind);
            if let Some(items) = arr.as_array() {
                for item in items {
                    findings.push(finding_from(item, cat));
                }
            }
        }
    }

    findings
}

fn finding_from(item: &serde_json::Value, category: Category) -> Finding {
    let advisory = item.get("advisory");
    let pkg = item.get("package");
    Finding {
        category,
        package: pkg
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("?")
            .to_string(),
        version: pkg
            .and_then(|p| p.get("version"))
            .and_then(|n| n.as_str())
            .unwrap_or("?")
            .to_string(),
        advisory_id: advisory
            .and_then(|a| a.get("id"))
            .and_then(|i| i.as_str())
            .map(str::to_string),
        title: advisory
            .and_then(|a| a.get("title"))
            .and_then(|t| t.as_str())
            .unwrap_or("(sem título)")
            .to_string(),
        url: advisory
            .and_then(|a| a.get("url"))
            .and_then(|u| u.as_str())
            .map(str::to_string),
        dependency_path: None,
    }
}

/// Classifica uma entrada da lista de vulnerabilidades pela metadata `informational`.
fn classify_advisory(advisory: Option<&serde_json::Value>) -> Category {
    match advisory
        .and_then(|a| a.get("informational"))
        .and_then(|i| i.as_str())
    {
        Some("unmaintained") => Category::Unmaintained,
        Some("unsound") => Category::Vulnerability,
        Some("notice") => Category::Notice,
        // `null`/ausente == vulnerabilidade a sério (inclui malware advisory).
        _ => Category::Vulnerability,
    }
}

fn kind_to_category(kind: &str) -> Category {
    match kind {
        "unmaintained" => Category::Unmaintained,
        "yanked" => Category::Yanked,
        "unsound" => Category::Vulnerability,
        "notice" => Category::Notice,
        _ => Category::Notice,
    }
}

/// Caminho de dependências inverso legível (`cargo tree -i`), a pedido.
pub fn inverse_dependency_path(
    manifest_dir: &Utf8Path,
    package: &str,
    version: &str,
) -> Option<String> {
    let spec = format!("{package}@{version}");
    let out = Command::new("cargo")
        .args(["tree", "-i", &spec, "-e", "normal,build"])
        .current_dir(manifest_dir)
        .output()
        .ok()?;
    if out.status.success() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_vuln_and_warnings() {
        let json = serde_json::json!({
            "vulnerabilities": { "found": true, "count": 1, "list": [
                {"advisory": {"id":"RUSTSEC-2024-0001","title":"Bang","url":"https://x","informational":null},
                 "package": {"name":"evil","version":"1.0.0"}}
            ]},
            "warnings": {
                "unmaintained": [
                    {"advisory":{"id":"RUSTSEC-2020-0002","title":"old"},"package":{"name":"stale","version":"0.1.0"}}
                ],
                "yanked": [
                    {"package":{"name":"gone","version":"2.0.0"}}
                ]
            }
        });
        let f = normalize(&json);
        assert_eq!(f.len(), 3);
        assert!(f
            .iter()
            .any(|x| x.package == "evil" && x.category == Category::Vulnerability));
        assert!(f
            .iter()
            .any(|x| x.package == "stale" && x.category == Category::Unmaintained));
        assert!(f
            .iter()
            .any(|x| x.package == "gone" && x.category == Category::Yanked));
    }

    #[test]
    fn unmaintained_informational_is_not_a_vulnerability() {
        let item = serde_json::json!({"advisory":{"informational":"unmaintained"}});
        assert_eq!(
            classify_advisory(item.get("advisory")),
            Category::Unmaintained
        );
    }
}
