//! Suites de teste configuráveis (`dx.toml`) + builtins + thresholds de carga.
//!
//! Contrato §8: não classificar suites só por nome sem configuração. `unit`,
//! `doc` e `integration` têm mapeamento Cargo embutido; qualquer outra suite
//! tem de estar declarada em `dx.toml`. Testes de carga falham por thresholds
//! (p95/p99/erros), não só por exit code.

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuiteKind {
    /// Saída libtest (cargo test) — parse por-teste.
    Rust,
    /// Comando genérico; sucesso/falha pelo exit code.
    Command,
    /// Teste de carga; falha por thresholds sobre métricas.
    Load,
}

#[derive(Debug, Clone)]
pub struct SuiteConfig {
    pub name: String,
    pub runner: Vec<String>,
    pub kind: SuiteKind,
    pub load: Option<LoadThresholds>,
}

/// Thresholds de um teste de carga (qualquer um excedido ⇒ falha).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LoadThresholds {
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
    pub max_error_rate: Option<f64>,
    pub min_rps: Option<f64>,
}

/// Métricas observadas (produzidas pelo gerador de carga, em JSON).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LoadMetrics {
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
    pub error_rate: Option<f64>,
    pub rps: Option<f64>,
}

/// Avalia as métricas contra os thresholds; devolve as violações (vazio = ok).
pub fn eval_load(t: &LoadThresholds, m: &LoadMetrics) -> Vec<String> {
    let mut v = Vec::new();
    if let (Some(lim), Some(obs)) = (t.p95_ms, m.p95_ms) {
        if obs > lim {
            v.push(format!("p95 {obs:.1}ms > {lim:.1}ms"));
        }
    }
    if let (Some(lim), Some(obs)) = (t.p99_ms, m.p99_ms) {
        if obs > lim {
            v.push(format!("p99 {obs:.1}ms > {lim:.1}ms"));
        }
    }
    if let (Some(lim), Some(obs)) = (t.max_error_rate, m.error_rate) {
        if obs > lim {
            v.push(format!("erros {:.2}% > {:.2}%", obs * 100.0, lim * 100.0));
        }
    }
    if let (Some(lim), Some(obs)) = (t.min_rps, m.rps) {
        if obs < lim {
            v.push(format!("rps {obs:.0} < {lim:.0}"));
        }
    }
    v
}

// --- dx.toml ---------------------------------------------------------------

#[derive(Deserialize)]
struct DxToml {
    test: Option<TestSection>,
}
#[derive(Deserialize)]
struct TestSection {
    suites: Option<BTreeMap<String, RawSuite>>,
}
#[derive(Deserialize)]
struct RawSuite {
    runner: Vec<String>,
    kind: Option<String>,
    load: Option<LoadThresholds>,
}

/// Lê as suites de um `dx.toml` (texto). Devolve mapa nome→config.
pub fn suites_from_dx_toml(text: &str) -> Result<BTreeMap<String, SuiteConfig>, toml::de::Error> {
    let parsed: DxToml = toml::from_str(text)?;
    let mut out = BTreeMap::new();
    if let Some(suites) = parsed.test.and_then(|t| t.suites) {
        for (name, raw) in suites {
            let kind = match raw.kind.as_deref() {
                Some("rust") => SuiteKind::Rust,
                Some("load") => SuiteKind::Load,
                _ => SuiteKind::Command,
            };
            out.insert(
                name.clone(),
                SuiteConfig {
                    name,
                    runner: raw.runner,
                    kind,
                    load: raw.load,
                },
            );
        }
    }
    Ok(out)
}

/// Suite embutida (mapeada a Cargo) para `unit`/`doc`/`integration`.
pub fn builtin(name: &str) -> Option<SuiteConfig> {
    let runner: Vec<String> = match name {
        "unit" => vec!["cargo", "test", "--lib", "--bins"],
        "doc" => vec!["cargo", "test", "--doc"],
        "integration" => vec!["cargo", "test", "--tests"],
        _ => return None,
    }
    .into_iter()
    .map(String::from)
    .collect();
    Some(SuiteConfig {
        name: name.to_string(),
        runner,
        kind: SuiteKind::Rust,
        load: None,
    })
}

/// Resolve uma suite: `dx.toml` tem precedência; senão builtin; senão `None`
/// (nome sem configuração não é classificado — contrato §8).
pub fn resolve(name: &str, dx_toml: Option<&str>) -> Result<Option<SuiteConfig>, toml::de::Error> {
    if let Some(text) = dx_toml {
        let map = suites_from_dx_toml(text)?;
        if let Some(cfg) = map.get(name) {
            return Ok(Some(cfg.clone()));
        }
    }
    Ok(builtin(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_map_to_cargo() {
        assert_eq!(
            builtin("unit").unwrap().runner,
            vec!["cargo", "test", "--lib", "--bins"]
        );
        assert!(builtin("inexistente").is_none());
    }

    #[test]
    fn dx_toml_overrides_and_adds() {
        let toml = r#"
[test.suites.e2e]
runner = ["./scripts/e2e.sh"]
kind = "command"

[test.suites.load]
runner = ["./scripts/load.sh"]
kind = "load"
[test.suites.load.load]
p95_ms = 200.0
max_error_rate = 0.01
"#;
        let e2e = resolve("e2e", Some(toml)).unwrap().unwrap();
        assert_eq!(e2e.kind, SuiteKind::Command);
        let load = resolve("load", Some(toml)).unwrap().unwrap();
        assert_eq!(load.kind, SuiteKind::Load);
        assert_eq!(load.load.unwrap().p95_ms, Some(200.0));
        // unit continua a resolver via builtin.
        assert!(resolve("unit", Some(toml)).unwrap().is_some());
        // nome desconhecido sem config → None.
        assert!(resolve("xyz", Some(toml)).unwrap().is_none());
    }

    #[test]
    fn load_thresholds_detect_violations() {
        let t = LoadThresholds {
            p95_ms: Some(100.0),
            max_error_rate: Some(0.01),
            ..Default::default()
        };
        let m = LoadMetrics {
            p95_ms: Some(150.0),
            error_rate: Some(0.05),
            ..Default::default()
        };
        let v = eval_load(&t, &m);
        assert_eq!(v.len(), 2, "p95 e erros violados: {v:?}");
    }

    #[test]
    fn load_thresholds_pass_when_within() {
        let t = LoadThresholds {
            p95_ms: Some(100.0),
            ..Default::default()
        };
        let m = LoadMetrics {
            p95_ms: Some(80.0),
            ..Default::default()
        };
        assert!(eval_load(&t, &m).is_empty());
    }
}
