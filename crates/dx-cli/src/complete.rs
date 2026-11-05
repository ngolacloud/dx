//! Completação dinâmica de shell para a CLI `dx`.
//!
//! Para além de comandos e flags — que vêm directamente da definição `clap` —
//! estes completadores resolvem, em tempo de execução, os VALORES que surgem a
//! seguir aos parâmetros: nomes de crates, templates, perfis de build e
//! suites/formatos de teste. As listas são derivadas do estado real do
//! projecto (`Cargo.toml`, `dx.toml`, cache do Cargo) e das próprias
//! bibliotecas do `dx` (`dx_build`, `dx_report`), para nunca divergirem do
//! comportamento real dos comandos.
//!
//! Regra de ouro: um completador NUNCA entra em pânico nem escreve para
//! stdout/stderr — na dúvida devolve menos candidatos.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use clap_complete::engine::{ArgValueCandidates, ArgValueCompleter, CompletionCandidate};

/// Templates de projecto (`dx new --template`, `dx init --template`).
///
/// Fonte: [`crate::scaffold::Template`]. Hoje só existe `seiva`.
pub fn templates() -> ArgValueCandidates {
    ArgValueCandidates::new(|| vec![cand("seiva", Some("Seiva platform (managed composition)"))])
}

/// Perfis de build (`dx build --profile`). Fonte: `dx_build::all_names`.
pub fn build_profile() -> ArgValueCandidates {
    ArgValueCandidates::new(|| {
        dx_build::all_names()
            .iter()
            .copied()
            .map(|name| cand(name, dx_build::builtin(name).map(|p| p.description)))
            .collect()
    })
}

/// Lista de perfis separada por vírgulas (`dx bench-build --profiles`).
///
/// Completa só o último segmento, preservando os perfis já escritos antes da
/// vírgula.
pub fn build_profiles_csv() -> ArgValueCompleter {
    ArgValueCompleter::new(|current: &OsStr| {
        let current = current.to_string_lossy();
        let (prefix, last) = split_csv(&current);
        dx_build::all_names()
            .iter()
            .copied()
            .filter(|n| n.starts_with(last))
            .map(|n| CompletionCandidate::new(format!("{prefix}{n}")))
            .collect()
    })
}

/// Formatos de relatório de teste (`dx test --format`). Fonte: `dx_report`.
pub fn test_format() -> ArgValueCandidates {
    ArgValueCandidates::new(|| {
        vec![
            cand("terminal", Some("human-readable terminal output")),
            cand("json", Some("machine-readable JSON")),
            cand("junit", Some("JUnit XML (CI)")),
            cand("html", Some("static HTML report")),
        ]
    })
}

/// Suites de teste (`dx test --suite`): builtin + as definidas em `dx.toml`.
pub fn test_suite() -> ArgValueCandidates {
    ArgValueCandidates::new(|| {
        let mut out = vec![
            cand("unit", Some("cargo test --lib --bins")),
            cand("doc", Some("cargo test --doc")),
            cand("integration", Some("cargo test --tests")),
        ];
        if let Some(text) = read_nearest("dx.toml") {
            if let Ok(map) = dx_report::suites_from_dx_toml(&text) {
                for name in map.keys() {
                    if !matches!(name.as_str(), "unit" | "doc" | "integration") {
                        out.push(cand(name, Some("suite defined in dx.toml")));
                    }
                }
            }
        }
        out
    })
}

/// Dependências já declaradas no manifesto (`dx remove`, `dx why`).
pub fn manifest_deps() -> ArgValueCompleter {
    ArgValueCompleter::new(|current: &OsStr| {
        let prefix = current.to_string_lossy();
        manifest_dep_names()
            .into_iter()
            .filter(|n| n.starts_with(prefix.as_ref()))
            .map(CompletionCandidate::new)
            .collect()
    })
}

/// Nomes de crates para `dx add`: dependências do manifesto + cache do Cargo.
///
/// Não há acesso ao índice completo do crates.io offline, por isso sugerem-se
/// as crates já conhecidas localmente — suficiente para a esmagadora maioria
/// dos `add` (versões anteriores, workspace, deps existentes).
pub fn crate_names() -> ArgValueCompleter {
    ArgValueCompleter::new(|current: &OsStr| {
        let token = current.to_string_lossy();
        // A completar a parte da versão (`nome@...`): não há nomes a sugerir.
        if token.contains('@') {
            return Vec::new();
        }
        let mut names: BTreeSet<String> = manifest_dep_names().into_iter().collect();
        names.extend(cached_crate_names());
        names
            .into_iter()
            .filter(|n| n.starts_with(token.as_ref()))
            .take(200)
            .map(CompletionCandidate::new)
            .collect()
    })
}

// --- auxiliares ------------------------------------------------------------

/// Constrói um candidato com ajuda opcional.
fn cand(value: &str, help: Option<&str>) -> CompletionCandidate {
    let c = CompletionCandidate::new(value);
    match help {
        Some(h) => c.help(Some(h.to_owned().into())),
        None => c,
    }
}

/// Divide um token CSV em `(prefixo-com-vírgula, último-segmento)`.
fn split_csv(current: &str) -> (&str, &str) {
    match current.rfind(',') {
        Some(i) => (&current[..=i], &current[i + 1..]),
        None => ("", current),
    }
}

/// Lê o ficheiro `name` mais próximo, subindo a partir do directório actual.
fn read_nearest(name: &str) -> Option<String> {
    let mut dir = std::env::current_dir().ok();
    while let Some(d) = dir {
        let candidate = d.join(name);
        if candidate.is_file() {
            return std::fs::read_to_string(candidate).ok();
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    None
}

/// Nomes das dependências declaradas no `Cargo.toml` mais próximo (incluindo
/// `dev`/`build` e as secções por-target).
fn manifest_dep_names() -> Vec<String> {
    let Some(text) = read_nearest("Cargo.toml") else {
        return Vec::new();
    };
    let Ok(value) = text.parse::<toml::Value>() else {
        return Vec::new();
    };
    let sections = ["dependencies", "dev-dependencies", "build-dependencies"];
    let mut names = BTreeSet::new();
    collect_dep_keys(&value, &sections, &mut names);
    if let Some(targets) = value.get("target").and_then(toml::Value::as_table) {
        for t in targets.values() {
            collect_dep_keys(t, &sections, &mut names);
        }
    }
    names.into_iter().collect()
}

/// Acumula as chaves das secções de dependências de uma tabela TOML.
fn collect_dep_keys(value: &toml::Value, sections: &[&str], out: &mut BTreeSet<String>) {
    for section in sections {
        if let Some(table) = value.get(*section).and_then(toml::Value::as_table) {
            out.extend(table.keys().cloned());
        }
    }
}

/// Nomes de crates presentes no cache local do Cargo (`registry/cache`).
fn cached_crate_names() -> Vec<String> {
    let Some(cache) = cargo_home().map(|h| h.join("registry").join("cache")) else {
        return Vec::new();
    };
    let Ok(registries) = std::fs::read_dir(&cache) else {
        return Vec::new();
    };
    let mut names = BTreeSet::new();
    for reg in registries.flatten() {
        let Ok(entries) = std::fs::read_dir(reg.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let file = entry.file_name();
            let file = file.to_string_lossy();
            if let Some(stem) = file.strip_suffix(".crate") {
                if let Some(name) = crate_name_from_stem(stem) {
                    names.insert(name.to_owned());
                }
            }
        }
    }
    names.into_iter().collect()
}

/// Localização do `CARGO_HOME` (ou `~/.cargo`).
fn cargo_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        return Some(PathBuf::from(home));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo"))
}

/// Extrai o nome da crate de um stem `nome-<versão>` (ex.: `md-5-0.10.6` →
/// `md-5`). A versão começa no último `-` seguido de dígito.
fn crate_name_from_stem(stem: &str) -> Option<&str> {
    let bytes = stem.as_bytes();
    for i in (0..bytes.len()).rev() {
        if bytes[i] == b'-' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
            return Some(&stem[..i]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_strips_version() {
        assert_eq!(crate_name_from_stem("serde-1.0.0"), Some("serde"));
        assert_eq!(
            crate_name_from_stem("error-chain-0.12.4"),
            Some("error-chain")
        );
        assert_eq!(
            crate_name_from_stem("x509-parser-0.16.0"),
            Some("x509-parser")
        );
        assert_eq!(crate_name_from_stem("md-5-0.10.6"), Some("md-5"));
        assert_eq!(crate_name_from_stem("clap-4.0.0-rc.1"), Some("clap"));
        assert_eq!(crate_name_from_stem("noversion"), None);
    }

    #[test]
    fn split_csv_keeps_prefix() {
        assert_eq!(split_csv("dev"), ("", "dev"));
        assert_eq!(split_csv("dev,rel"), ("dev,", "rel"));
        assert_eq!(split_csv("dev,release,"), ("dev,release,", ""));
    }
}
