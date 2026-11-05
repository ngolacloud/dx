//! Parse da saída humana do libtest (`cargo test`) em casos.
//!
//! As linhas por-teste do libtest são estáveis em stable: `test <nome> ... ok`
//! / `FAILED` / `ignored`. Com `--report-time` surge `<N.NNNs>` no fim, que
//! aproveitamos para a duração. Não dependemos de nightly nem de JSON instável.

use crate::model::{Case, Status};

/// Faz parse de todas as linhas por-teste da saída combinada de `cargo test`.
pub fn parse_libtest(output: &str) -> Vec<Case> {
    let mut cases = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        let rest = match line.strip_prefix("test ") {
            Some(r) => r,
            None => continue,
        };
        // Ignorar o resumo "test result: ...".
        if rest.starts_with("result:") {
            continue;
        }
        let (name, tail) = match rest.rsplit_once(" ... ") {
            Some(x) => x,
            None => continue,
        };
        let (status, duration_ms) = classify(tail);
        if let Some(status) = status {
            cases.push(Case {
                name: name.to_string(),
                status,
                duration_ms,
                message: None,
            });
        }
    }
    cases
}

fn classify(tail: &str) -> (Option<Status>, Option<u64>) {
    // tail pode ser "ok", "FAILED", "ignored", ou "ok <0.001s>".
    let mut parts = tail.splitn(2, ' ');
    let word = parts.next().unwrap_or("");
    let dur = parts.next().and_then(parse_report_time);
    let status = match word {
        "ok" => Some(Status::Passed),
        "FAILED" => Some(Status::Failed),
        "ignored" => Some(Status::Ignored),
        _ => None,
    };
    (status, dur)
}

/// Extrai milissegundos de `<0.001s>`.
fn parse_report_time(s: &str) -> Option<u64> {
    let inner = s.trim().strip_prefix('<')?.strip_suffix('>')?;
    let secs = inner.strip_suffix('s')?.trim().parse::<f64>().ok()?;
    Some((secs * 1000.0).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ok_failed_ignored() {
        let out = "
running 3 tests
test a::passes ... ok
test a::fails ... FAILED
test a::skip ... ignored
test result: FAILED. 1 passed; 1 failed; 1 ignored;
";
        let cases = parse_libtest(out);
        assert_eq!(cases.len(), 3);
        assert_eq!(cases[0].status, Status::Passed);
        assert_eq!(cases[1].status, Status::Failed);
        assert_eq!(cases[2].status, Status::Ignored);
    }

    #[test]
    fn parses_report_time() {
        let out = "test a::x ... ok <0.012s>\n";
        let cases = parse_libtest(out);
        assert_eq!(cases[0].duration_ms, Some(12));
    }

    #[test]
    fn ignores_non_test_lines() {
        assert!(parse_libtest("Compiling foo\nFinished\n").is_empty());
    }
}
