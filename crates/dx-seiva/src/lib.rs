//! `dx-seiva` — adaptador Seiva.
//!
//! Protocolo **declarativo e versionado** de integração, interpretado por
//! código **confiável do DX** (este crate). NUNCA corre hooks/scripts fornecidos
//! pela extensão (contrato §6): `dx add` de uma extensão Seiva conhecida apenas
//! edita ficheiros geridos por **âncoras** estáveis. A integração é idempotente
//! e reversível, não concede permissões administrativas nem cria passwords.
//!
//! A geração de módulos de negócio/contratos/migrações continua a pertencer à
//! CLI/biblioteca Seiva; o DX só faz a cablagem (dependência + âncora + registo).

use serde::Serialize;

/// Versão do protocolo de integração (o spec é do DX, não vem do crate).
pub const PROTOCOL_VERSION: u32 = 1;

/// Especificação declarativa de uma extensão Seiva conhecida.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtensionSpec {
    pub protocol_version: u32,
    /// Nome do crate (ex.: `seiva-console-admin`).
    pub crate_name: &'static str,
    /// Feature opcional a activar no package de composição.
    pub feature: Option<&'static str>,
    /// Linha a inserir na âncora `use` (import/âncora compilada), se precisa.
    pub use_line: Option<&'static str>,
    /// Linha a inserir na âncora `register` (registo de plugin).
    pub register_line: &'static str,
    /// Descrição humana do que a integração faz.
    pub summary: &'static str,
}

/// Registo confiável de extensões conhecidas. O nome canónico é
/// `seiva-console-admin` (NUNCA instalar um parecido — ver typosquatting).
pub fn known(name: &str) -> Option<ExtensionSpec> {
    match name {
        "seiva-console-admin" => Some(ExtensionSpec {
            protocol_version: PROTOCOL_VERSION,
            crate_name: "seiva-console-admin",
            feature: None,
            use_line: None,
            register_line: "seiva_console_admin::register();",
            summary: "registers the admin console in the Seiva composition",
        }),
        _ => None,
    }
}

pub fn is_known(name: &str) -> bool {
    known(name).is_some()
}

/// Nome de módulo Rust do crate (hífens → underscores).
pub fn crate_module(crate_name: &str) -> String {
    crate_name.replace('-', "_")
}

// --- Integração por âncoras -----------------------------------------------

/// Resultado de aplicar um spec a um ficheiro de composição.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationResult {
    pub new_source: String,
    pub changed: bool,
    /// Âncoras em falta (composição não gerida para estas regiões).
    pub missing_anchors: Vec<String>,
}

fn begin(region: &str) -> String {
    format!("// dx:seiva:{region}:begin")
}
fn end(region: &str) -> String {
    format!("// dx:seiva:{region}:end")
}

/// Verifica se o ficheiro tem as âncoras de uma região.
pub fn has_region(src: &str, region: &str) -> bool {
    src.contains(&begin(region)) && src.contains(&end(region))
}

/// Garante que `line` existe (exactamente uma vez) dentro da região, preservando
/// indentação do marcador. Idempotente. Devolve (novo_src, mudou).
fn upsert_in_region(src: &str, region: &str, line: &str) -> Option<(String, bool)> {
    let b = begin(region);
    let e = end(region);
    let lines: Vec<&str> = src.lines().collect();
    let bi = lines.iter().position(|l| l.trim() == b)?;
    let ei = lines.iter().position(|l| l.trim() == e)?;
    if ei <= bi {
        return None;
    }
    // Indentação a partir do marcador begin.
    let indent: String = lines[bi]
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    let managed = format!("{indent}{line}");

    // Já presente (comparando sem espaços à direita)?
    let present = lines[bi + 1..ei].iter().any(|l| l.trim() == line.trim());
    if present {
        return Some((src.to_string(), false));
    }
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 1);
    out.extend(lines[..=bi].iter().map(|s| s.to_string()));
    out.extend(lines[bi + 1..ei].iter().map(|s| s.to_string()));
    out.push(managed);
    out.extend(lines[ei..].iter().map(|s| s.to_string()));
    let mut joined = out.join("\n");
    if src.ends_with('\n') {
        joined.push('\n');
    }
    Some((joined, true))
}

/// Remove `line` de uma região (reversibilidade). Devolve (novo_src, mudou).
fn remove_from_region(src: &str, region: &str, line: &str) -> Option<(String, bool)> {
    let b = begin(region);
    let e = end(region);
    let lines: Vec<&str> = src.lines().collect();
    let bi = lines.iter().position(|l| l.trim() == b)?;
    let ei = lines.iter().position(|l| l.trim() == e)?;
    if ei <= bi {
        return None;
    }
    let mut changed = false;
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for (i, l) in lines.iter().enumerate() {
        if i > bi && i < ei && l.trim() == line.trim() {
            changed = true;
            continue;
        }
        out.push(l.to_string());
    }
    let mut joined = out.join("\n");
    if src.ends_with('\n') {
        joined.push('\n');
    }
    Some((joined, changed))
}

/// Aplica um spec ao ficheiro de composição (idempotente).
pub fn integrate(src: &str, spec: &ExtensionSpec) -> IntegrationResult {
    let mut cur = src.to_string();
    let mut changed = false;
    let mut missing = Vec::new();

    if let Some(use_line) = spec.use_line {
        match upsert_in_region(&cur, "use", use_line) {
            Some((s, c)) => {
                cur = s;
                changed |= c;
            }
            None => missing.push("use".to_string()),
        }
    }
    match upsert_in_region(&cur, "register", spec.register_line) {
        Some((s, c)) => {
            cur = s;
            changed |= c;
        }
        None => missing.push("register".to_string()),
    }

    IntegrationResult {
        new_source: cur,
        changed,
        missing_anchors: missing,
    }
}

/// Reverte um spec (remove as linhas geridas).
pub fn revert(src: &str, spec: &ExtensionSpec) -> IntegrationResult {
    let mut cur = src.to_string();
    let mut changed = false;
    if let Some(use_line) = spec.use_line {
        if let Some((s, c)) = remove_from_region(&cur, "use", use_line) {
            cur = s;
            changed |= c;
        }
    }
    if let Some((s, c)) = remove_from_region(&cur, "register", spec.register_line) {
        cur = s;
        changed |= c;
    }
    IntegrationResult {
        new_source: cur,
        changed,
        missing_anchors: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMP: &str = "\
pub fn build_app() {
    // dx:seiva:use:begin
    // dx:seiva:use:end

    // dx:seiva:register:begin
    // dx:seiva:register:end
}
";

    #[test]
    fn integrates_and_is_idempotent() {
        let spec = known("seiva-console-admin").unwrap();
        let r1 = integrate(COMP, &spec);
        assert!(r1.changed);
        assert!(r1.missing_anchors.is_empty());
        assert!(r1.new_source.contains("seiva_console_admin::register();"));
        // Segunda vez: no-op (idempotente).
        let r2 = integrate(&r1.new_source, &spec);
        assert!(!r2.changed, "repetir integração não deve mudar nada");
        assert_eq!(r1.new_source, r2.new_source);
        // Só uma ocorrência.
        assert_eq!(r2.new_source.matches("register();").count(), 1);
    }

    #[test]
    fn revert_removes_managed_line() {
        let spec = known("seiva-console-admin").unwrap();
        let r = integrate(COMP, &spec);
        let back = revert(&r.new_source, &spec);
        assert!(back.changed);
        assert!(!back.new_source.contains("register();"));
        // As âncoras permanecem.
        assert!(has_region(&back.new_source, "register"));
    }

    #[test]
    fn missing_anchors_reported_not_faked() {
        let spec = known("seiva-console-admin").unwrap();
        let unmanaged = "pub fn build_app() {}\n";
        let r = integrate(unmanaged, &spec);
        assert!(!r.changed);
        assert_eq!(r.missing_anchors, vec!["register"]);
    }

    #[test]
    fn preserves_marker_indentation() {
        let spec = known("seiva-console-admin").unwrap();
        let r = integrate(COMP, &spec);
        assert!(r
            .new_source
            .contains("    seiva_console_admin::register();"));
    }
}
