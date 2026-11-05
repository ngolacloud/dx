//! Detecção de typosquatting por proximidade a nomes conhecidos.
//!
//! Isto é um SINAL para exigir confirmação, nunca prova de malícia nem de
//! benignidade (contrato §5). A lista é deliberadamente pequena no inc.1:
//! extensões oficiais do ecossistema + alguns crates muito populares. No
//! inc.2 passa a cruzar com o índice/política da organização.

use dx_core::levenshtein;

/// Nomes "âncora" contra os quais medimos proximidade.
const KNOWN: &[&str] = &[
    // Extensões oficiais Seiva (o nome canónico é seiva-console-admin).
    "seiva-console-admin",
    "seiva-cli",
    "seiva-http",
    // Crates muito populares, alvos típicos de typosquatting.
    "serde",
    "serde_json",
    "tokio",
    "anyhow",
    "thiserror",
    "clap",
    "rand",
    "reqwest",
    "regex",
];

/// Se `name` não é conhecido mas está a 1–2 edições de um nome conhecido,
/// devolve a sugestão e a distância. Nomes exactos nunca disparam.
pub fn suspect_typo(name: &str) -> Option<(String, usize)> {
    if KNOWN.contains(&name) {
        return None;
    }
    let mut best: Option<(&str, usize)> = None;
    for &k in KNOWN {
        let d = levenshtein(name, k);
        if d == 0 {
            return None;
        }
        if d <= 2 {
            match best {
                Some((_, bd)) if bd <= d => {}
                _ => best = Some((k, d)),
            }
        }
    }
    best.map(|(s, d)| (s.to_string(), d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_console_admin_typo() {
        let (s, _) = suspect_typo("seiva-conosle-admin").expect("deve sugerir");
        assert_eq!(s, "seiva-console-admin");
    }

    #[test]
    fn exact_name_is_not_suspicious() {
        assert!(suspect_typo("seiva-console-admin").is_none());
        assert!(suspect_typo("serde").is_none());
    }

    #[test]
    fn unrelated_name_is_fine() {
        assert!(suspect_typo("minha-lib-unica-xyz").is_none());
    }
}
