//! `dx-report` — modelo de relatório de testes e emissores.
//!
//! O modelo é independente do runner; os runners (ver dx-cli) preenchem um
//! `Report` e escolhem o emissor. Invariantes do contrato §8: erro de runner
//! nunca vira suite verde (campo `ok`), HTML escapado e segredos removidos.

pub mod emit;
pub mod model;
pub mod parse;
pub mod suite;

pub use emit::{html, html_escape, json, junit, scrub, terminal};
pub use model::{Case, Report, Status, Suite};
pub use parse::parse_libtest;
pub use suite::{
    builtin as builtin_suite, eval_load, resolve as resolve_suite, suites_from_dx_toml,
    LoadMetrics, LoadThresholds, SuiteConfig, SuiteKind,
};

/// Formatos de saída suportados.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Terminal,
    Json,
    Junit,
    Html,
}

impl Format {
    pub fn parse(s: &str) -> Option<Format> {
        Some(match s {
            "terminal" | "text" => Format::Terminal,
            "json" => Format::Json,
            "junit" | "xml" => Format::Junit,
            "html" => Format::Html,
            _ => return None,
        })
    }

    /// Emite o relatório no formato. `ansi` só afecta o terminal.
    pub fn render(&self, report: &Report, ansi: bool) -> String {
        match self {
            Format::Terminal => terminal(report, ansi),
            Format::Json => json(report),
            Format::Junit => junit(report),
            Format::Html => html(report),
        }
    }

    pub fn default_extension(&self) -> &'static str {
        match self {
            Format::Terminal => "txt",
            Format::Json => "json",
            Format::Junit => "xml",
            Format::Html => "html",
        }
    }
}
