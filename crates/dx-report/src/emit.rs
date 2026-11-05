//! Emissores de relatório: terminal, JSON versionado, JUnit, HTML.
//!
//! Dois invariantes do contrato §8: **escapar HTML** e **remover segredos** das
//! mensagens antes de as emitir.

use crate::model::{Report, Status};

/// Remove segredos óbvios de uma mensagem antes de a reportar.
/// Heurística conservadora: redige o valor após palavras-chave sensíveis.
pub fn scrub(s: &str) -> String {
    const KEYS: &[&str] = &[
        "secret",
        "token",
        "password",
        "passwd",
        "api_key",
        "apikey",
        "api-key",
        "authorization",
        "bearer",
    ];
    const RED: &str = "***REDACTED***";
    let is_key = |t: &str| {
        let t = t.trim_end_matches([':', '=']).to_ascii_lowercase();
        KEYS.contains(&t.as_str())
    };

    let mut lines = Vec::new();
    for line in s.lines() {
        let tokens: Vec<&str> = line.split(' ').collect();
        let mut redact_next = false;
        let mut rebuilt: Vec<String> = Vec::with_capacity(tokens.len());
        for tok in tokens {
            if redact_next {
                // `password: VALOR` / `Bearer VALOR`.
                rebuilt.push(if tok.is_empty() {
                    tok.to_string()
                } else {
                    RED.to_string()
                });
                if !tok.is_empty() {
                    redact_next = false;
                }
                continue;
            }
            // `key=valor` ou `key:valor` dentro do mesmo token.
            if let Some(sep) = tok.find(['=', ':']) {
                let key = &tok[..sep];
                if KEYS.contains(&key.to_ascii_lowercase().as_str()) {
                    let value = &tok[sep + 1..];
                    if value.is_empty() {
                        // `password:` → o valor é o próximo token.
                        rebuilt.push(tok.to_string());
                        redact_next = true;
                    } else {
                        let sepch = &tok[sep..sep + 1];
                        rebuilt.push(format!("{key}{sepch}{RED}"));
                    }
                    continue;
                }
            }
            // `key:` / `key` isolado → redige o próximo token.
            if is_key(tok) {
                rebuilt.push(tok.to_string());
                redact_next = true;
                continue;
            }
            rebuilt.push(tok.to_string());
        }
        lines.push(rebuilt.join(" "));
    }
    lines.join("\n")
}

/// Escapa texto para HTML.
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Escapa texto para atributo/conteúdo XML (JUnit).
fn xml_escape(s: &str) -> String {
    html_escape(s)
}

/// Relatório legível em terminal. `ansi` liga cores (modo sem-ANSI = texto puro).
pub fn terminal(report: &Report, ansi: bool) -> String {
    let (green, red, yellow, dim, reset) = if ansi {
        ("\x1b[32m", "\x1b[31m", "\x1b[33m", "\x1b[2m", "\x1b[0m")
    } else {
        ("", "", "", "", "")
    };
    let mut out = String::new();
    for s in &report.suites {
        out.push_str(&format!("suite {} ({})\n", s.name, s.runner.join(" ")));
        if let Some(err) = &s.runner_error {
            out.push_str(&format!("  {red}erro de runner:{reset} {}\n", scrub(err)));
        }
        for c in &s.cases {
            let (mark, col) = match c.status {
                Status::Passed => ("ok   ", green),
                Status::Failed => ("FALHA", red),
                Status::Ignored => ("ign  ", yellow),
                Status::NotRun => ("n/exe", dim),
                Status::Flaky => ("flaky", yellow),
            };
            let dur = c
                .duration_ms
                .map(|d| format!(" {dim}({d}ms){reset}"))
                .unwrap_or_default();
            out.push_str(&format!("  {col}{mark}{reset} {}{dur}\n", c.name));
            if c.status == Status::Failed {
                if let Some(m) = &c.message {
                    out.push_str(&format!("        {}\n", scrub(m)));
                }
            }
        }
        out.push_str(&format!(
            "  → {}{} passed{}, {}{} failed{}, {} ignored, {} flaky {}\n",
            green,
            s.passed(),
            reset,
            red,
            s.failed(),
            reset,
            s.ignored(),
            s.flaky(),
            if s.ok && s.failed() == 0 {
                format!("{green}[OK]{reset}")
            } else {
                format!("{red}[FALHA]{reset}")
            },
        ));
    }
    out
}

/// JSON versionado (schemaVersion no topo). Mensagens são scrubbed — o JSON
/// é um relatório como os outros e não pode vazar segredos.
pub fn json(report: &Report) -> String {
    let mut r = report.clone();
    for s in &mut r.suites {
        if let Some(e) = &s.runner_error {
            s.runner_error = Some(scrub(e));
        }
        for c in &mut s.cases {
            if let Some(m) = &c.message {
                c.message = Some(scrub(m));
            }
        }
    }
    serde_json::to_string_pretty(&r).unwrap_or_else(|_| "{}".into())
}

/// JUnit XML.
pub fn junit(report: &Report) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites>\n");
    for s in &report.suites {
        out.push_str(&format!(
            "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" skipped=\"{}\">\n",
            xml_escape(&s.name),
            s.cases.len(),
            s.failed(),
            s.ignored(),
        ));
        if let Some(err) = &s.runner_error {
            out.push_str(&format!(
                "    <testcase name=\"__runner__\"><error message=\"runner\">{}</error></testcase>\n",
                xml_escape(&scrub(err))
            ));
        }
        for c in &s.cases {
            let time = c
                .duration_ms
                .map(|d| format!(" time=\"{:.3}\"", d as f64 / 1000.0))
                .unwrap_or_default();
            out.push_str(&format!(
                "    <testcase name=\"{}\"{time}>",
                xml_escape(&c.name)
            ));
            match c.status {
                Status::Failed => out.push_str(&format!(
                    "<failure>{}</failure>",
                    xml_escape(&scrub(c.message.as_deref().unwrap_or("")))
                )),
                Status::Ignored | Status::NotRun => out.push_str("<skipped/>"),
                _ => {}
            }
            out.push_str("</testcase>\n");
        }
        out.push_str("  </testsuite>\n");
    }
    out.push_str("</testsuites>\n");
    out
}

/// HTML auto-contido (sem recursos externos), escapado e sem segredos.
pub fn html(report: &Report) -> String {
    let mut body = String::new();
    for s in &report.suites {
        let status_class = if s.ok && s.failed() == 0 {
            "ok"
        } else {
            "fail"
        };
        body.push_str(&format!(
            "<section class=\"suite {status_class}\"><h2>{}</h2><p class=\"runner\">{}</p>",
            html_escape(&s.name),
            html_escape(&s.runner.join(" "))
        ));
        if let Some(err) = &s.runner_error {
            body.push_str(&format!(
                "<pre class=\"err\">{}</pre>",
                html_escape(&scrub(err))
            ));
        }
        body.push_str(&format!(
            "<p class=\"counts\">{} passed · {} failed · {} ignored · {} flaky</p><ul>",
            s.passed(),
            s.failed(),
            s.ignored(),
            s.flaky()
        ));
        for c in &s.cases {
            let dur = c
                .duration_ms
                .map(|d| format!(" <span class=dim>({d}ms)</span>"))
                .unwrap_or_default();
            let msg = if c.status == Status::Failed {
                c.message
                    .as_deref()
                    .map(|m| format!("<pre>{}</pre>", html_escape(&scrub(m))))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            body.push_str(&format!(
                "<li class=\"{}\">{} — {}{dur}{msg}</li>",
                c.status.as_str(),
                html_escape(&c.name),
                c.status.as_str()
            ));
        }
        body.push_str("</ul></section>");
    }
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>dx test report</title>\
         <style>body{{font-family:system-ui,sans-serif;margin:2rem;max-width:60rem}}\
         .suite{{border:1px solid #ccc;border-radius:8px;padding:1rem;margin:1rem 0}}\
         .suite.ok h2::before{{content:'✓ ';color:green}}.suite.fail h2::before{{content:'✗ ';color:#c00}}\
         .runner{{color:#666;font-family:monospace;font-size:.85em}}\
         li.failed{{color:#c00}}li.ignored{{color:#a60}}li.passed{{color:#060}}\
         .dim{{color:#999}}pre.err{{background:#fee;padding:.5rem;overflow:auto}}</style></head>\
         <body><h1>dx test report</h1><p>schema v{} · {}</p>{}</body></html>",
        report.schema_version,
        html_escape(&report.generated),
        body
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Case, Report, Suite};

    fn sample() -> Report {
        let mut r = Report::new("2026-11-05T00:00:00Z");
        r.suites.push(Suite {
            name: "unit".into(),
            runner: vec!["cargo".into(), "test".into()],
            cases: vec![
                Case {
                    name: "a::ok".into(),
                    status: Status::Passed,
                    duration_ms: Some(3),
                    message: None,
                },
                Case {
                    name: "a::bad".into(),
                    status: Status::Failed,
                    duration_ms: None,
                    message: Some("token=abc123 falhou em <x>".into()),
                },
            ],
            duration_ms: Some(10),
            ok: true,
            runner_error: None,
        });
        r
    }

    #[test]
    fn scrub_redacts_secrets() {
        assert_eq!(scrub("token=abc123"), "token=***REDACTED***");
        assert!(scrub("password: hunter2").ends_with("***REDACTED***"));
        assert_eq!(scrub("nada aqui"), "nada aqui");
    }

    #[test]
    fn html_escapes_and_scrubs() {
        let h = html(&sample());
        assert!(!h.contains("abc123"), "segredo não pode aparecer no HTML");
        assert!(!h.contains("<x>"), "o `<x>` tem de ser escapado");
        assert!(h.contains("&lt;x&gt;"));
        assert!(h.contains("<!doctype html>"));
    }

    #[test]
    fn json_is_versioned() {
        let j = json(&sample());
        assert!(j.contains("\"schema_version\": 1"));
        assert!(
            !j.contains("abc123"),
            "JSON é relatório: não pode vazar segredos"
        );
    }

    #[test]
    fn junit_has_failure_and_escapes() {
        let x = junit(&sample());
        assert!(x.contains("<testsuite name=\"unit\""));
        assert!(x.contains("<failure>"));
        assert!(!x.contains("abc123"));
        assert!(x.contains("&lt;x&gt;"));
    }

    #[test]
    fn terminal_plain_has_no_ansi() {
        let t = terminal(&sample(), false);
        assert!(!t.contains('\x1b'));
        assert!(t.contains("FALHA"));
    }
}
