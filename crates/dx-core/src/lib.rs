//! `dx-core` — modelos de domínio puros do DX, sem I/O.
//!
//! Esta crate não toca em ficheiros, rede nem processos: é a fronteira
//! testável onde vivem a identidade de packages, as especificações de
//! dependências e os planos de alteração.

pub mod error;
pub mod model;
pub mod plan;

pub use error::{DxError, Result};
pub use model::{DepKind, DepSpec, PackageId, Source};
pub use plan::{Change, Plan};

/// Distância de edição de Levenshtein — base da detecção de typosquatting
/// (p.ex. `seiva-conosle-admin` vs `seiva-console-admin`). Não prova malícia;
/// é um sinal para exigir confirmação, nunca para substituir em silêncio.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DepSpec, Source};

    #[test]
    fn levenshtein_catches_single_transposition() {
        assert_eq!(levenshtein("conosle", "console"), 2);
        assert_eq!(levenshtein("serde", "serde"), 0);
        assert_eq!(levenshtein("tokio", "tokyo"), 1);
    }

    #[test]
    fn parse_name_at_version() {
        assert_eq!(
            DepSpec::parse_name_at_version("serde").unwrap(),
            ("serde".into(), None)
        );
        assert_eq!(
            DepSpec::parse_name_at_version("serde@1.2").unwrap(),
            ("serde".into(), Some("1.2".into()))
        );
        assert!(DepSpec::parse_name_at_version("serde@").is_err());
        assert!(DepSpec::parse_name_at_version("").is_err());
    }

    #[test]
    fn features_normalize_for_idempotent_compare() {
        let mut a = DepSpec::registry("serde", Some("1".into()));
        a.features = vec!["derive".into(), "alloc".into(), "derive".into()];
        let mut b = DepSpec::registry("serde", Some("1".into()));
        b.features = vec!["alloc".into(), "derive".into()];
        assert!(
            a.same_state(&b),
            "features devem comparar independentes da ordem/dups"
        );
    }

    #[test]
    fn different_source_is_not_same_state() {
        let reg = DepSpec::registry("foo", Some("1".into()));
        let mut git = DepSpec::registry("foo", Some("1".into()));
        git.source = Source::Git {
            url: "https://x/y".into(),
            rev: Some("abc".into()),
            tag: None,
            branch: None,
        };
        assert!(!reg.same_state(&git));
        assert!(!reg.source.same_origin(&git.source));
    }
}
