//! Integração com `cargo-deny` (origens, licenças, bans) — complementa o
//! `cargo-audit` (advisories). Reutilizamos a ferramenta madura em vez de
//! reimplementar a verificação de licenças/origens.
//!
//! `cargo-deny` emite NDJSON (`--format json`): uma linha por diagnóstico
//! (`type":"diagnostic"`) e uma de sumário (`type":"summary"`). Mapeamos cada
//! diagnóstico de severidade `error`/`warning` para um `Finding` categorizado.
//!
//! Só corremos `licenses sources bans` — os advisories ficam para o
//! `cargo-audit`, evitando dupla contagem.

use camino::Utf8Path;
use std::io::Write;
use std::process::Command;

use crate::audit::Finding;
use crate::policy::Category;

/// `deny.toml` embutido, usado quando o projecto não traz o seu.
pub const DEFAULT_DENY_TOML: &str = include_str!("../assets/default-deny.toml");

#[derive(Debug, thiserror::Error)]
pub enum DenyError {
    #[error("não consegui invocar `cargo deny`: {0}")]
    Spawn(#[from] std::io::Error),
}

/// `cargo-deny` está instalado?
pub fn available() -> bool {
    Command::new("cargo")
        .args(["deny", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Corre `cargo deny check licenses sources bans` em `manifest_dir`.
/// Usa o `deny.toml` do projecto se existir; senão escreve o embutido num
/// ficheiro temporário e passa-o por `--config`.
///
/// Devolve os achados mapeados. Se o `cargo-deny` não estiver disponível,
/// devolve vazio (o chamador regista a falta de cobertura — não finge).
pub fn run(manifest_dir: &Utf8Path) -> Result<Vec<Finding>, DenyError> {
    if !available() {
        return Ok(Vec::new());
    }

    // Config: do projecto, ou o embutido num temporário.
    let project_cfg = manifest_dir.join("deny.toml");
    let tmp_cfg;
    let cfg_path = if project_cfg.exists() {
        project_cfg
    } else {
        let mut f = tempfile::NamedTempFile::new()?;
        f.write_all(DEFAULT_DENY_TOML.as_bytes())?;
        let (_, path) = f.keep().map_err(|e| e.error)?;
        tmp_cfg = camino::Utf8PathBuf::from_path_buf(path)
            .unwrap_or_else(|_| camino::Utf8PathBuf::from("deny.toml"));
        tmp_cfg.clone()
    };

    let out = Command::new("cargo")
        .args(["deny", "--format", "json", "--config", cfg_path.as_str()])
        .args(["check", "licenses", "sources", "bans"])
        .current_dir(manifest_dir)
        .output()?;

    // O NDJSON sai por stderr. Código != 0 quando há erros (esperado).
    let text = String::from_utf8_lossy(&out.stderr);
    Ok(parse_ndjson(&text))
}

/// Faz parse do NDJSON do cargo-deny em achados.
fn parse_ndjson(text: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue, // linhas não-JSON (ex.: progresso) ignoradas
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("diagnostic") {
            continue; // ignora summary e outros
        }
        let fields = match v.get("fields") {
            Some(f) => f,
            None => continue,
        };
        let severity = fields
            .get("severity")
            .and_then(|s| s.as_str())
            .unwrap_or("");
        if severity != "error" && severity != "warning" {
            continue; // help/note não são achados accionáveis
        }
        let message = fields
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("(sem mensagem)")
            .to_string();
        let code = fields.get("code").and_then(|c| c.as_str()).unwrap_or("");
        let (pkg, ver) = root_krate(fields);
        findings.push(Finding {
            category: classify(code, &message),
            package: pkg,
            version: ver,
            advisory_id: if code.is_empty() {
                None
            } else {
                Some(format!("deny:{code}"))
            },
            title: message,
            url: None,
            dependency_path: None,
        });
    }
    findings
}

/// Extrai o crate raiz de `fields.graphs[0].Krate`.
fn root_krate(fields: &serde_json::Value) -> (String, String) {
    let krate = fields
        .get("graphs")
        .and_then(|g| g.as_array())
        .and_then(|a| a.first())
        .and_then(|g| g.get("Krate"));
    let name = krate
        .and_then(|k| k.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("?")
        .to_string();
    let version = krate
        .and_then(|k| k.get("version"))
        .and_then(|n| n.as_str())
        .unwrap_or("?")
        .to_string();
    (name, version)
}

/// Classifica um diagnóstico do cargo-deny numa categoria do DX.
fn classify(code: &str, message: &str) -> Category {
    let c = code.to_ascii_lowercase();
    let m = message.to_ascii_lowercase();
    if c.contains("registry")
        || c.contains("source")
        || m.contains("source") && m.contains("not allowed")
    {
        return Category::SourceNotAllowed;
    }
    if c.contains("git") && (c.contains("unknown") || c.contains("not-allowed")) {
        return Category::SourceNotAllowed;
    }
    if m.contains("license") || c == "rejected" {
        return Category::License;
    }
    if c.contains("ban")
        || c.contains("wildcard")
        || c.contains("duplicate")
        || m.contains("banned")
    {
        return Category::Ban;
    }
    Category::Notice
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_license_and_source_diagnostics() {
        let ndjson = concat!(
            r#"{"type":"diagnostic","fields":{"code":"rejected","severity":"error","message":"failed to satisfy license requirements","graphs":[{"Krate":{"name":"foo","version":"1.0.0"}}]}}"#,
            "\n",
            r#"{"type":"diagnostic","fields":{"code":"unknown-registry","severity":"error","message":"crate from unknown registry","graphs":[{"Krate":{"name":"bar","version":"2.0.0"}}]}}"#,
            "\n",
            r#"{"type":"summary","fields":{"licenses":{"errors":1}}}"#,
        );
        let f = parse_ndjson(ndjson);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].category, Category::License);
        assert_eq!(f[0].package, "foo");
        assert_eq!(f[1].category, Category::SourceNotAllowed);
        assert_eq!(f[1].package, "bar");
    }

    #[test]
    fn skips_help_and_notes() {
        let ndjson = r#"{"type":"diagnostic","fields":{"code":"x","severity":"help","message":"h","graphs":[]}}"#;
        assert!(parse_ndjson(ndjson).is_empty());
    }

    #[test]
    fn default_deny_toml_is_embedded() {
        assert!(DEFAULT_DENY_TOML.contains("[sources]"));
        assert!(DEFAULT_DENY_TOML.contains("unknown-registry"));
    }
}
