//! Modelo de relatório de testes (independente do runner).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Passed,
    Failed,
    Ignored,
    /// Declarado mas não executado (ex.: suite abortou antes).
    NotRun,
    /// Passou só após repetição (precisa de runner com retry; ver limites).
    Flaky,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Passed => "passed",
            Status::Failed => "failed",
            Status::Ignored => "ignored",
            Status::NotRun => "not-run",
            Status::Flaky => "flaky",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Case {
    pub name: String,
    pub status: Status,
    pub duration_ms: Option<u64>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suite {
    pub name: String,
    /// argv do runner (para rastreabilidade).
    pub runner: Vec<String>,
    pub cases: Vec<Case>,
    pub duration_ms: Option<u64>,
    /// `true` só se o runner terminou bem **e** não há casos a falhar.
    /// Erro de runner nunca vira suite verde (contrato §8).
    pub ok: bool,
    /// Mensagem de erro do runner (compilação falhou, etc.), se aplicável.
    pub runner_error: Option<String>,
}

impl Suite {
    pub fn count(&self, status: Status) -> usize {
        self.cases.iter().filter(|c| c.status == status).count()
    }
    pub fn passed(&self) -> usize {
        self.count(Status::Passed)
    }
    pub fn failed(&self) -> usize {
        self.count(Status::Failed)
    }
    pub fn ignored(&self) -> usize {
        self.count(Status::Ignored)
    }
    pub fn flaky(&self) -> usize {
        self.count(Status::Flaky)
    }
}

/// Relatório completo. `schema_version` é o contrato versionado do JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub generated: String,
    pub suites: Vec<Suite>,
}

impl Report {
    pub const SCHEMA_VERSION: u32 = 1;

    pub fn new(generated: impl Into<String>) -> Self {
        Report {
            schema_version: Self::SCHEMA_VERSION,
            generated: generated.into(),
            suites: Vec::new(),
        }
    }

    /// Relatório "verde"? Todas as suites ok e sem falhas.
    pub fn ok(&self) -> bool {
        self.suites.iter().all(|s| s.ok && s.failed() == 0)
    }
}
