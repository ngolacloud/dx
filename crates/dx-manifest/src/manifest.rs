//! Leitura e mutação de Cargo.toml preservando formatação e comentários.
//!
//! Autoridade: Cargo.toml/Cargo.lock continuam autoritativos (contrato §2).
//! Esta camada só edita a tabela de dependências visada, deixando o resto do
//! documento (ordem, comentários, espaçamento) intacto — é o que o toml_edit
//! garante ao mexermos apenas numa chave.

use camino::{Utf8Path, Utf8PathBuf};
use dx_core::{Change, DepKind, DepSpec, DxError, Source};
use toml_edit::{Array, DocumentMut, Formatted, InlineTable, Item, Table, Value};

/// Erros de I/O/parse do manifesto (distintos dos erros de domínio em dx-core).
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("could not read {path}: {source}")]
    Read {
        path: Utf8PathBuf,
        source: std::io::Error,
    },
    #[error("invalid Cargo.toml at {path}: {source}")]
    Parse {
        path: Utf8PathBuf,
        source: toml_edit::TomlError,
    },
    #[error(transparent)]
    Domain(#[from] DxError),
}

pub type Result<T> = std::result::Result<T, ManifestError>;

/// Um Cargo.toml carregado e editável.
pub struct Manifest {
    pub path: Utf8PathBuf,
    doc: DocumentMut,
}

impl Manifest {
    pub fn load(path: impl AsRef<Utf8Path>) -> Result<Manifest> {
        let path = path.as_ref().to_owned();
        let text = std::fs::read_to_string(&path).map_err(|source| ManifestError::Read {
            path: path.clone(),
            source,
        })?;
        let doc = text
            .parse::<DocumentMut>()
            .map_err(|source| ManifestError::Parse {
                path: path.clone(),
                source,
            })?;
        Ok(Manifest { path, doc })
    }

    /// Serializa o documento actual (preservando formatação).
    pub fn to_toml_string(&self) -> String {
        self.doc.to_string()
    }

    /// Nome do package (se `[package] name`).
    pub fn package_name(&self) -> Option<&str> {
        self.doc
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|v| v.as_str())
    }

    /// Navega (sem criar) até à tabela de dependências de uma dada secção.
    fn table_ref(&self, kind: DepKind, target: Option<&str>) -> Option<&Item> {
        match target {
            None => self.doc.get(kind.table_key()),
            Some(t) => self
                .doc
                .get("target")
                .and_then(|x| x.get(t))
                .and_then(|x| x.get(kind.table_key())),
        }
    }

    /// Lê a especificação declarada de uma dependência pela sua chave TOML.
    pub fn find(&self, entry_key: &str, kind: DepKind, target: Option<&str>) -> Option<DepSpec> {
        let table = self.table_ref(kind, target)?;
        let item = table.get(entry_key)?;
        Some(item_to_spec(entry_key, item, kind, target))
    }

    /// Calcula a alteração necessária para convergir para `spec`.
    /// - `None`  → já convergido (no-op, idempotente).
    /// - `Add`   → não existe.
    /// - `Update`→ existe, mesma origem, estado diferente (versão/features...).
    /// - erro    → existe com origem diferente (mudança de origem é explícita).
    pub fn plan_add(&self, spec: &DepSpec) -> Result<Option<Change>> {
        let mut want = spec.clone();
        want.normalize();
        let manifest = self.path.to_string();
        match self.find(want.table_entry_key(), want.kind, want.target.as_deref()) {
            None => Ok(Some(Change::AddDependency {
                manifest,
                spec: want,
            })),
            Some(mut existing) => {
                existing.normalize();
                if existing.same_state(&want) {
                    Ok(None) // idempotente
                } else if existing.source.same_origin(&want.source) {
                    Ok(Some(Change::UpdateDependency {
                        manifest,
                        from: Box::new(existing),
                        to: Box::new(want),
                    }))
                } else {
                    Err(DxError::SourceConflict {
                        name: want.name.clone(),
                        existing: existing.source.to_string(),
                        requested: want.source.to_string(),
                    }
                    .into())
                }
            }
        }
    }

    /// Calcula a remoção. `None` se a dependência não existe (no-op).
    pub fn plan_remove(
        &self,
        entry_key: &str,
        kind: DepKind,
        target: Option<&str>,
    ) -> Option<Change> {
        self.find(entry_key, kind, target)
            .map(|_| Change::RemoveDependency {
                manifest: self.path.to_string(),
                key: entry_key.to_string(),
                table: kind.table_key().to_string(),
            })
    }

    /// Aplica `spec` ao documento (insere ou substitui a chave).
    pub fn upsert(&mut self, spec: &DepSpec) {
        let mut spec = spec.clone();
        spec.normalize();
        let item = spec_to_item(&spec);
        let table = self.table_mut(spec.kind, spec.target.as_deref());
        table.insert(spec.table_entry_key(), item);
    }

    /// Remove uma chave de dependência. Devolve `true` se existia.
    pub fn remove(&mut self, entry_key: &str, kind: DepKind, target: Option<&str>) -> bool {
        let table = match self.table_mut_opt(kind, target) {
            Some(t) => t,
            None => return false,
        };
        table.remove(entry_key).is_some()
    }

    fn table_mut(&mut self, kind: DepKind, target: Option<&str>) -> &mut Table {
        match target {
            None => dep_table(self.doc.as_table_mut(), kind.table_key()),
            Some(t) => {
                let target_tbl = sub_table(self.doc.as_table_mut(), "target");
                let one = sub_table(target_tbl, t);
                dep_table(one, kind.table_key())
            }
        }
    }

    fn table_mut_opt(&mut self, kind: DepKind, target: Option<&str>) -> Option<&mut Table> {
        match target {
            None => self
                .doc
                .get_mut(kind.table_key())
                .and_then(Item::as_table_mut),
            Some(t) => self
                .doc
                .get_mut("target")
                .and_then(|x| x.get_mut(t))
                .and_then(|x| x.get_mut(kind.table_key()))
                .and_then(Item::as_table_mut),
        }
    }
}

fn sub_table<'a>(parent: &'a mut Table, key: &str) -> &'a mut Table {
    parent
        .entry(key)
        .or_insert_with(|| Item::Table(Table::new()))
        .as_table_mut()
        .expect("table key")
}

fn dep_table<'a>(parent: &'a mut Table, key: &str) -> &'a mut Table {
    sub_table(parent, key)
}

/// Converte um item TOML existente numa `DepSpec` normalizada.
fn item_to_spec(entry_key: &str, item: &Item, kind: DepKind, target: Option<&str>) -> DepSpec {
    // Forma curta: `foo = "1.2"`.
    if let Some(v) = item.as_str() {
        let mut s = DepSpec::registry(entry_key, Some(v.to_string()));
        s.kind = kind;
        s.target = target.map(str::to_string);
        s.normalize();
        return s;
    }

    // Forma de tabela (inline `{ ... }` ou `[deps.foo]`).
    let get_str = |k: &str| item.get(k).and_then(|x| x.as_str()).map(str::to_string);
    let get_bool = |k: &str| item.get(k).and_then(|x| x.as_bool());
    let features = item
        .get("features")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let package = get_str("package");
    let (name, rename) = match package {
        Some(real) => (real, Some(entry_key.to_string())),
        None => (entry_key.to_string(), None),
    };

    let source = if let Some(url) = get_str("git") {
        Source::Git {
            url,
            rev: get_str("rev"),
            tag: get_str("tag"),
            branch: get_str("branch"),
        }
    } else if let Some(path) = get_str("path") {
        Source::Path { path }
    } else {
        Source::Registry {
            registry: get_str("registry"),
        }
    };

    let mut s = DepSpec {
        name,
        version_req: get_str("version"),
        source,
        features,
        default_features: get_bool("default-features").unwrap_or(true),
        optional: get_bool("optional").unwrap_or(false),
        kind,
        target: target.map(str::to_string),
        rename,
    };
    s.normalize();
    s
}

/// Constrói o item TOML para uma `DepSpec`. Usa a forma curta quando possível
/// (só versão de registry por omissão), senão uma inline table determinística.
fn spec_to_item(spec: &DepSpec) -> Item {
    let is_plain_registry = matches!(spec.source, Source::Registry { registry: None });
    let minimal = is_plain_registry
        && spec.features.is_empty()
        && spec.default_features
        && !spec.optional
        && spec.rename.is_none();

    if minimal {
        if let Some(v) = &spec.version_req {
            return Item::Value(Value::String(Formatted::new(v.clone())));
        }
    }

    let mut t = InlineTable::new();
    match &spec.source {
        Source::Registry { registry } => {
            if let Some(v) = &spec.version_req {
                t.insert("version", v.as_str().into());
            }
            if let Some(r) = registry {
                t.insert("registry", r.as_str().into());
            }
        }
        Source::Git {
            url,
            rev,
            tag,
            branch,
        } => {
            t.insert("git", url.as_str().into());
            if let Some(r) = rev {
                t.insert("rev", r.as_str().into());
            } else if let Some(tg) = tag {
                t.insert("tag", tg.as_str().into());
            } else if let Some(b) = branch {
                t.insert("branch", b.as_str().into());
            }
            if let Some(v) = &spec.version_req {
                t.insert("version", v.as_str().into());
            }
        }
        Source::Path { path } => {
            t.insert("path", path.as_str().into());
            if let Some(v) = &spec.version_req {
                t.insert("version", v.as_str().into());
            }
        }
    }
    if let Some(real) = &spec.rename {
        // rename: a chave é o alias local; `package` aponta ao crate real.
        let _ = real;
        t.insert("package", spec.name.as_str().into());
    }
    if !spec.features.is_empty() {
        let mut arr = Array::new();
        for f in &spec.features {
            arr.push(f.as_str());
        }
        t.insert("features", Value::Array(arr));
    }
    if !spec.default_features {
        t.insert("default-features", false.into());
    }
    if spec.optional {
        t.insert("optional", true.into());
    }
    Item::Value(Value::InlineTable(t))
}
