//! Política de segurança: categorias distintas e acção por categoria.
//!
//! Nunca confundir `yanked`, `unmaintained` e `vulnerability` (contrato §5):
//! são categorias separadas com acção explícita. `malware`, `integrity` e
//! `source-not-allowed` são bloqueios duros por omissão.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Categorias de achado. Deliberadamente distintas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// Advisory de vulnerabilidade (RUSTSEC, com ou sem CVE).
    Vulnerability,
    /// Malware conhecido. Bloqueio duro.
    Malware,
    /// Versão retirada (yanked).
    Yanked,
    /// Crate sem manutenção.
    Unmaintained,
    /// Falha de integridade (checksum/arquivo). Bloqueio duro.
    IntegrityFailure,
    /// Origem não permitida (ex.: fallback público de package privado). Duro.
    SourceNotAllowed,
    /// Aviso informacional sem categoria específica.
    Notice,
}

impl Category {
    pub fn as_str(&self) -> &'static str {
        match self {
            Category::Vulnerability => "vulnerability",
            Category::Malware => "malware",
            Category::Yanked => "yanked",
            Category::Unmaintained => "unmaintained",
            Category::IntegrityFailure => "integrity-failure",
            Category::SourceNotAllowed => "source-not-allowed",
            Category::Notice => "notice",
        }
    }

    /// Bloqueios duros não são enfraquecíveis por `dx.toml` desconhecido.
    pub fn is_hard_block(&self) -> bool {
        matches!(
            self,
            Category::Malware | Category::IntegrityFailure | Category::SourceNotAllowed
        )
    }
}

/// Acção a aplicar quando uma categoria aparece.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Block,
    Warn,
    Allow,
}

/// Validade máxima da base de advisories, em segundos.
pub const DEFAULT_MAX_DB_AGE_DEV_SECS: u64 = 24 * 60 * 60;

/// Política de segurança efectiva.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub actions: BTreeMap<Category, Action>,
    /// Idade máxima da base em segundos (dev).
    pub max_db_age_secs: u64,
    /// Exigir refresh obrigatório antes de release (aplicado no inc.4).
    pub refresh_before_release: bool,
}

impl Default for Policy {
    /// Baseline do ADR-0005.
    fn default() -> Self {
        let mut actions = BTreeMap::new();
        actions.insert(Category::Vulnerability, Action::Block);
        actions.insert(Category::Malware, Action::Block);
        actions.insert(Category::IntegrityFailure, Action::Block);
        actions.insert(Category::SourceNotAllowed, Action::Block);
        actions.insert(Category::Yanked, Action::Warn);
        actions.insert(Category::Unmaintained, Action::Warn);
        actions.insert(Category::Notice, Action::Warn);
        Policy {
            actions,
            max_db_age_secs: DEFAULT_MAX_DB_AGE_DEV_SECS,
            refresh_before_release: true,
        }
    }
}

impl Policy {
    /// Política de desenvolvimento, com a validade da base sobreponível por
    /// `DX_MAX_DB_AGE_SECS` (o contrato pede validade máxima configurável).
    /// `0` torna qualquer base "expirada"; um valor alto alarga a janela.
    pub fn from_env() -> Self {
        let mut p = Policy::default();
        if let Ok(v) = std::env::var("DX_MAX_DB_AGE_SECS") {
            if let Ok(secs) = v.trim().parse::<u64>() {
                p.max_db_age_secs = secs;
            }
        }
        p
    }

    pub fn action_for(&self, cat: Category) -> Action {
        // Bloqueios duros nunca descem abaixo de Block, mesmo que a config
        // tente enfraquecê-los.
        if cat.is_hard_block() {
            return Action::Block;
        }
        self.actions.get(&cat).copied().unwrap_or(Action::Warn)
    }

    /// Endurece a política combinando com outra (o mais apertado vence).
    /// Usado para impedir que um `dx.toml` desconhecido enfraqueça a política
    /// da organização.
    pub fn harden_with(&mut self, other: &Policy) {
        for (cat, act) in &other.actions {
            let current = self.action_for(*cat);
            let tighter = tightest(current, *act);
            self.actions.insert(*cat, tighter);
        }
        self.max_db_age_secs = self.max_db_age_secs.min(other.max_db_age_secs);
        self.refresh_before_release |= other.refresh_before_release;
    }

    /// Hash estável da política, para a chave do cache de decisão.
    pub fn fingerprint(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        let json = serde_json::to_vec(self).unwrap_or_default();
        h.update(&json);
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
}

fn tightest(a: Action, b: Action) -> Action {
    use Action::*;
    match (a, b) {
        (Block, _) | (_, Block) => Block,
        (Warn, _) | (_, Warn) => Warn,
        _ => Allow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_blocks_cannot_be_weakened() {
        let mut p = Policy::default();
        // Tentar enfraquecer malware para Allow não tem efeito.
        p.actions.insert(Category::Malware, Action::Allow);
        assert_eq!(p.action_for(Category::Malware), Action::Block);
    }

    #[test]
    fn harden_takes_the_tighter() {
        let mut org = Policy::default();
        org.actions.insert(Category::Yanked, Action::Block);
        let mut repo = Policy::default(); // yanked = Warn
        repo.harden_with(&org);
        assert_eq!(repo.action_for(Category::Yanked), Action::Block);
    }
}
