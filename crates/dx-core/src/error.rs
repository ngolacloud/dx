//! Erros de domínio do DX.

use thiserror::Error;

/// Erro de domínio. A camada de I/O (dx-manifest) e a CLI (dx-cli) acrescentam
/// contexto por cima destes; aqui ficam apenas as falhas conceptuais.
#[derive(Debug, Error)]
pub enum DxError {
    /// A dependência pedida existe mas com uma origem diferente da configurada.
    /// Mudar de origem é sempre explícito — nunca um efeito lateral de `add`.
    #[error(
        "dependency `{name}` is already configured with a different source \
         (configured: {existing}; requested: {requested}); \
         changing the source requires removing it explicitly first"
    )]
    SourceConflict {
        name: String,
        existing: String,
        requested: String,
    },

    /// Nome de package que parece erro de escrita de um nome conhecido.
    /// Nunca instalar silenciosamente o parecido — sugere a correcção.
    #[error("suspicious name `{typo}`: did you mean `{suggestion}`? (edit distance {distance})")]
    SuspiciousName {
        typo: String,
        suggestion: String,
        distance: usize,
    },

    /// Falta informação para identificar inequivocamente um package.
    #[error("ambiguous identification of `{name}`: {reason}")]
    AmbiguousPackage { name: String, reason: String },

    /// Pediu-se uma versão de registry mas não foi possível resolver.
    #[error("no version for `{name}`: {reason}")]
    UnresolvedVersion { name: String, reason: String },

    /// Especificação malformada vinda da CLI.
    #[error("invalid specification: {0}")]
    InvalidSpec(String),
}

pub type Result<T> = std::result::Result<T, DxError>;
