//! Estatísticas de amostras (p50/p95) — nearest-rank, determinístico.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub n: usize,
    pub p50: f64,
    pub p95: f64,
    pub mean: f64,
    pub min: f64,
    pub max: f64,
}

/// Percentil nearest-rank (P em 0..=100). `samples` não precisa de estar ordenado.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    // rank = ceil(p/100 * n), 1-indexed, clamp a [1, n].
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    let idx = rank.clamp(1, sorted.len()) - 1;
    sorted[idx]
}

impl Stats {
    pub fn from_samples(samples: &[f64]) -> Stats {
        let mut s = samples.to_vec();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = s.len();
        let sum: f64 = s.iter().sum();
        Stats {
            n,
            p50: percentile(&s, 50.0),
            p95: percentile(&s, 95.0),
            mean: if n == 0 { 0.0 } else { sum / n as f64 },
            min: s.first().copied().unwrap_or(0.0),
            max: s.last().copied().unwrap_or(0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_nearest_rank() {
        let s: Vec<f64> = (1..=10).map(|x| x as f64).collect();
        let st = Stats::from_samples(&s);
        assert_eq!(st.p50, 5.0); // ceil(0.5*10)=5 → idx4 → 5
        assert_eq!(st.p95, 10.0); // ceil(0.95*10)=10 → idx9 → 10
        assert_eq!(st.min, 1.0);
        assert_eq!(st.max, 10.0);
        assert_eq!(st.mean, 5.5);
    }

    #[test]
    fn handles_unsorted_and_single() {
        let st = Stats::from_samples(&[9.0, 1.0, 5.0]);
        assert_eq!(st.min, 1.0);
        assert_eq!(st.max, 9.0);
        assert_eq!(Stats::from_samples(&[42.0]).p95, 42.0);
    }
}
