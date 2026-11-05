//! Erros de domínio do DX.

use thiserror::Error;

/// Erro de domínio. A camada de I/O (dx-manifest) e a CLI (dx-cli) acrescentam
/// contexto por cima destes; aqui ficam apenas as falhas conceptuais.
#[derive(Debug, Error)]
pub enum DxError {
    /// A dependência pedida existe mas com uma origem diferente da configurada.
    /// Mudar de origem é sempre explícito — nunca um efeito lateral de `add`.
    #[error(
        "a dependência `{name}` já está configurada com origem diferente \
         (configurada: {existing}; pedida: {requested}); \
         mudar de origem exige remoção explícita primeiro"
    )]
    SourceConflict {
        name: String,
        existing: String,
        requested: String,
    },

    /// Nome de package que parece erro de escrita de um nome conhecido.
    /// Nunca instalar silenciosamente o parecido — sugere a correcção.
    #[error("nome suspeito `{typo}`: quis dizer `{suggestion}`? (distância de edição {distance})")]
    SuspiciousName {
        typo: String,
        suggestion: String,
        distance: usize,
    },

    /// Falta informação para identificar inequivocamente um package.
    #[error("identificação ambígua de `{name}`: {reason}")]
    AmbiguousPackage { name: String, reason: String },

    /// Pediu-se uma versão de registry mas não foi possível resolver.
    #[error("sem versão para `{name}`: {reason}")]
    UnresolvedVersion { name: String, reason: String },

    /// Especificação malformada vinda da CLI.
    #[error("especificação inválida: {0}")]
    InvalidSpec(String),
}

pub type Result<T> = std::result::Result<T, DxError>;
