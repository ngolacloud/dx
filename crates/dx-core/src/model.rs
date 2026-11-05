//! Modelos de identidade e especificação de dependências.
//!
//! Princípio inegociável (contrato §2): um package identifica-se por
//! **nome + versão + origem + revisão/checksum**, nunca só pelo nome.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::error::{DxError, Result};

/// Origem de uma dependência. Reflecte o que o Cargo distingue:
/// registry (crates.io ou alternativo), git (com commit fixado) e path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// Registry. `None` == registry por omissão (crates.io).
    Registry { registry: Option<String> },
    /// Dependência Git. A revisão fixada (`rev`) é a identidade forte;
    /// `tag`/`branch` são ponteiros que o lockfile resolve para um commit.
    Git {
        url: String,
        rev: Option<String>,
        tag: Option<String>,
        branch: Option<String>,
    },
    /// Dependência local por caminho. A identidade forte é a fingerprint
    /// de conteúdo (calculada pela camada de I/O), não o caminho.
    Path { path: String },
}

impl Source {
    pub fn crates_io() -> Self {
        Source::Registry { registry: None }
    }

    /// Dois `Source` são "a mesma origem" para efeitos de idempotência?
    /// Para git, um `rev` explícito diferente é origem diferente; tag/branch
    /// iguais também contam. Para registry, o nome do registry tem de bater.
    pub fn same_origin(&self, other: &Source) -> bool {
        match (self, other) {
            (Source::Registry { registry: a }, Source::Registry { registry: b }) => a == b,
            (
                Source::Git {
                    url: ua,
                    rev: ra,
                    tag: ta,
                    branch: ba,
                },
                Source::Git {
                    url: ub,
                    rev: rb,
                    tag: tb,
                    branch: bb,
                },
            ) => ua == ub && ra == rb && ta == tb && ba == bb,
            (Source::Path { path: a }, Source::Path { path: b }) => a == b,
            _ => false,
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Registry { registry: None } => write!(f, "registry:crates.io"),
            Source::Registry {
                registry: Some(name),
            } => write!(f, "registry:{name}"),
            Source::Git {
                url,
                rev,
                tag,
                branch,
            } => {
                write!(f, "git:{url}")?;
                if let Some(r) = rev {
                    write!(f, "#rev={r}")?;
                } else if let Some(t) = tag {
                    write!(f, "#tag={t}")?;
                } else if let Some(b) = branch {
                    write!(f, "#branch={b}")?;
                }
                Ok(())
            }
            Source::Path { path } => write!(f, "path:{path}"),
        }
    }
}

/// Secção da tabela de dependências em Cargo.toml.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

impl DepKind {
    /// Nome da tabela TOML correspondente (sem prefixo de target).
    pub fn table_key(&self) -> &'static str {
        match self {
            DepKind::Normal => "dependencies",
            DepKind::Dev => "dev-dependencies",
            DepKind::Build => "build-dependencies",
        }
    }
}

/// Especificação declarada de uma dependência — o que o utilizador pretende.
/// Corresponde ao que é escrito/lido em Cargo.toml, de forma normalizada para
/// comparação idempotente.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepSpec {
    /// Nome do crate (a chave real, mesmo quando há `package = "..."`/rename).
    pub name: String,
    /// Requisito de versão SemVer (para registry/git opcional). `None` quando
    /// a origem não é registry.
    pub version_req: Option<String>,
    pub source: Source,
    /// Features explicitamente activadas, **normalizadas** (ordenadas, sem
    /// duplicados) para que a comparação de idempotência seja estável.
    pub features: Vec<String>,
    pub default_features: bool,
    pub optional: bool,
    pub kind: DepKind,
    /// `cfg(...)`/triple de target, p.ex. `cfg(unix)`; `None` == todos.
    pub target: Option<String>,
    /// Nome local quando difere do crate (`foo = { package = "bar" }`).
    pub rename: Option<String>,
}

impl DepSpec {
    /// Construtor mínimo para uma dependência de registry por omissão.
    pub fn registry(name: impl Into<String>, version_req: Option<String>) -> Self {
        DepSpec {
            name: name.into(),
            version_req,
            source: Source::crates_io(),
            features: Vec::new(),
            default_features: true,
            optional: false,
            kind: DepKind::Normal,
            target: None,
            rename: None,
        }
    }

    /// Chave TOML sob a qual a dependência aparece (rename quando existe).
    pub fn table_entry_key(&self) -> &str {
        self.rename.as_deref().unwrap_or(&self.name)
    }

    /// Normaliza features: ordena e remove duplicados. Idempotência depende
    /// disto — `--features a,b` e `--features b,a,a` são o mesmo estado.
    pub fn normalize(&mut self) {
        self.features.sort();
        self.features.dedup();
    }

    /// Dois specs representam o MESMO estado desejado?
    /// Usado para decidir se `dx add` é no-op. Compara tudo o que define o
    /// estado — incluindo features normalizadas e default-features.
    pub fn same_state(&self, other: &DepSpec) -> bool {
        let mut a = self.clone();
        let mut b = other.clone();
        a.normalize();
        b.normalize();
        a == b
    }

    /// Faz parse de uma especificação de CLI no formato `nome` ou `nome@req`.
    pub fn parse_name_at_version(spec: &str) -> Result<(String, Option<String>)> {
        if let Some((name, req)) = spec.split_once('@') {
            if name.is_empty() {
                return Err(DxError::InvalidSpec(format!("empty name in `{spec}`")));
            }
            if req.is_empty() {
                return Err(DxError::InvalidSpec(format!(
                    "empty version requirement in `{spec}`"
                )));
            }
            Ok((name.to_string(), Some(req.to_string())))
        } else {
            if spec.is_empty() {
                return Err(DxError::InvalidSpec("empty name".into()));
            }
            Ok((spec.to_string(), None))
        }
    }
}

/// Identidade **resolvida** de um package no grafo — o que seguiu para o
/// staging/lockfile. É isto que o gate de segurança avalia.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageId {
    pub name: String,
    pub version: String,
    pub source: Source,
    /// Checksum do artefacto de registry (sha256 hex) quando aplicável.
    pub checksum: Option<String>,
    /// Commit resolvido, para origens git.
    pub resolved_rev: Option<String>,
}

impl fmt::Display for PackageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{} ({})", self.name, self.version, self.source)
    }
}
