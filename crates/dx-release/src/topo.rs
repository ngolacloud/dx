//! Ordem topológica de publicação de um workspace.
//!
//! Para publicar um workspace, os crates têm de sair na ordem das dependências
//! (um crate só se publica depois dos que consome) — contrato §8.

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Um membro do workspace e as suas dependências **internas** (outros membros).
#[derive(Debug, Clone, Serialize)]
pub struct Member {
    pub name: String,
    pub internal_deps: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TopoError {
    #[error("dependency cycle among members: {0:?}")]
    Cycle(Vec<String>),
}

/// Ordena os membros topologicamente (dependências primeiro).
/// Determinístico: desempata por ordem alfabética.
pub fn publish_order(members: &[Member]) -> Result<Vec<String>, TopoError> {
    let names: BTreeSet<&str> = members.iter().map(|m| m.name.as_str()).collect();
    // Grafo só com arestas para membros existentes.
    let mut deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for m in members {
        let e = deps.entry(m.name.clone()).or_default();
        for d in &m.internal_deps {
            if names.contains(d.as_str()) && d != &m.name {
                e.insert(d.clone());
            }
        }
    }

    let mut order = Vec::new();
    let mut done: BTreeSet<String> = BTreeSet::new();
    // Kahn determinístico: repetir até estabilizar; detectar ciclo.
    while order.len() < deps.len() {
        let mut progressed = false;
        for (name, d) in &deps {
            if done.contains(name) {
                continue;
            }
            if d.iter().all(|x| done.contains(x)) {
                order.push(name.clone());
                done.insert(name.clone());
                progressed = true;
            }
        }
        if !progressed {
            let remaining: Vec<String> = deps
                .keys()
                .filter(|k| !done.contains(*k))
                .cloned()
                .collect();
            return Err(TopoError::Cycle(remaining));
        }
    }
    Ok(order)
}

/// Faz parse da saída de `cargo metadata --no-deps --format-version 1` (JSON)
/// e devolve os membros com as suas dependências internas.
pub fn members_from_metadata(json: &str) -> Result<Vec<Member>, serde_json::Error> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let names: BTreeSet<String> = v
        .get("packages")
        .and_then(|p| p.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|p| p.get("name").and_then(|n| n.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut members = Vec::new();
    if let Some(pkgs) = v.get("packages").and_then(|p| p.as_array()) {
        for p in pkgs {
            let name = match p.get("name").and_then(|n| n.as_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let internal_deps = p
                .get("dependencies")
                .and_then(|d| d.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|d| d.get("name").and_then(|n| n.as_str()))
                        .filter(|d| names.contains(*d))
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();
            members.push(Member {
                name,
                internal_deps,
            });
        }
    }
    Ok(members)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str, deps: &[&str]) -> Member {
        Member {
            name: name.into(),
            internal_deps: deps.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn deps_come_before_dependents() {
        let members = vec![
            m("dx-cli", &["dx-core", "dx-manifest"]),
            m("dx-manifest", &["dx-core"]),
            m("dx-core", &[]),
        ];
        let order = publish_order(&members).unwrap();
        let pos = |n: &str| order.iter().position(|x| x == n).unwrap();
        assert!(pos("dx-core") < pos("dx-manifest"));
        assert!(pos("dx-manifest") < pos("dx-cli"));
    }

    #[test]
    fn detects_cycle() {
        let members = vec![m("a", &["b"]), m("b", &["a"])];
        assert!(matches!(publish_order(&members), Err(TopoError::Cycle(_))));
    }

    #[test]
    fn parses_metadata() {
        let json = r#"{"packages":[
            {"name":"dx-core","dependencies":[]},
            {"name":"dx-manifest","dependencies":[{"name":"dx-core"},{"name":"serde"}]}
        ]}"#;
        let members = members_from_metadata(json).unwrap();
        let mani = members.iter().find(|m| m.name == "dx-manifest").unwrap();
        assert_eq!(mani.internal_deps, vec!["dx-core"]); // serde não é membro
    }
}
