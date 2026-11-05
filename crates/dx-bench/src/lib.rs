//! `dx-bench` — harness de benchmarks do pipeline de dependências.
//!
//! Mede **resolução** (cargo) vs **análise/gate** (DX), com cache frio/quente,
//! p50/p95 e memória, sobre um **mock registry** determinista (crates por
//! `path`). O orçamento de overhead é reportado contra uma meta de engenharia
//! (≤10% em cache quente), **nunca** desactivando verificações para a cumprir.
//!
//! Cobertura honesta: download real, rede rápida/lenta e amostra real
//! controlada NÃO estão neste harness (grafo por path não descarrega) — ver
//! docs/benchmarks.md.

pub mod mock;
pub mod stats;

pub use mock::gen_mock_workspace;
pub use stats::Stats;

use serde::Serialize;

/// Resultado de uma dimensão (tamanho N) do benchmark.
#[derive(Debug, Clone, Serialize)]
pub struct SizeResult {
    pub packages: usize,
    pub resolve: Stats,
    pub gate_cold: Stats,
    pub gate_warm: Stats,
    pub max_rss_kb: Option<u64>,
    /// Overhead do gate em cache quente, em % sobre a resolução.
    pub warm_overhead_pct: f64,
}

impl SizeResult {
    /// Calcula o overhead quente (gate_warm.p50 / resolve.p50 * 100).
    pub fn with_overhead(mut self) -> Self {
        self.warm_overhead_pct = if self.resolve.p50 > 0.0 {
            self.gate_warm.p50 / self.resolve.p50 * 100.0
        } else {
            0.0
        };
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overhead_computed() {
        let r = SizeResult {
            packages: 100,
            resolve: Stats::from_samples(&[100.0, 100.0]),
            gate_cold: Stats::from_samples(&[50.0]),
            gate_warm: Stats::from_samples(&[5.0, 5.0]),
            max_rss_kb: None,
            warm_overhead_pct: 0.0,
        }
        .with_overhead();
        assert!((r.warm_overhead_pct - 5.0).abs() < 1e-9, "5ms/100ms = 5%");
    }
}
