//! Plano de alteração: o que o DX *vai* fazer antes de tocar em ficheiros.
//!
//! Todas as operações locais produzem um `Plan` primeiro (contrato §4). O
//! `--dry-run` imprime-o sem aplicar. Aplicar um plano vazio é no-op.

use crate::model::DepSpec;
use std::fmt;

/// Uma alteração atómica a aplicar ao projecto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Criar um ficheiro novo (scaffold). `contents` é o conteúdo final.
    CreateFile { path: String, contents: String },
    /// Acrescentar uma dependência a uma tabela do manifesto.
    AddDependency { manifest: String, spec: DepSpec },
    /// Substituir uma dependência existente (mesma chave, estado diferente).
    UpdateDependency {
        manifest: String,
        from: Box<DepSpec>,
        to: Box<DepSpec>,
    },
    /// Remover uma dependência.
    RemoveDependency {
        manifest: String,
        key: String,
        table: String,
    },
}

impl Change {
    /// Resumo de uma linha, legível, para o terminal.
    pub fn summary(&self) -> String {
        match self {
            Change::CreateFile { path, .. } => format!("criar  {path}"),
            Change::AddDependency { manifest, spec } => format!(
                "add    {} = {} [{}] em {manifest}:{}",
                spec.table_entry_key(),
                spec.version_req.as_deref().unwrap_or("*"),
                spec.source,
                spec.kind.table_key(),
            ),
            Change::UpdateDependency { manifest, from, to } => format!(
                "update {} : {} -> {} em {manifest}",
                to.table_entry_key(),
                from.source,
                to.source,
            ),
            Change::RemoveDependency {
                manifest,
                key,
                table,
            } => format!("remove {key} de {manifest}:{table}"),
        }
    }
}

/// Conjunto ordenado de alterações. A ordem é a de aplicação.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub changes: Vec<Change>,
}

impl Plan {
    pub fn new() -> Self {
        Plan::default()
    }

    pub fn push(&mut self, change: Change) {
        self.changes.push(change);
    }

    /// Um plano sem alterações representa um estado já convergido (no-op).
    pub fn is_noop(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.changes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

impl fmt::Display for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_noop() {
            return write!(f, "(sem alterações — já convergido)");
        }
        for (i, c) in self.changes.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "  {}", c.summary())?;
        }
        Ok(())
    }
}
