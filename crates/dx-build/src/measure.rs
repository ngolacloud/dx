//! Medição de builds: tempo (relógio), memória máxima e tamanho do binário.
//!
//! Tempo: `std::time::Instant` à volta do processo. Memória máxima: GNU
//! `/usr/bin/time -v` (campo "Maximum resident set size"), quando disponível —
//! senão reportamos `None` (não inventamos). Tamanho: bytes do artefacto.

use camino::Utf8Path;
use serde::Serialize;
use std::process::Command;
use std::time::Instant;

#[derive(Debug, Clone, Serialize)]
pub struct Measurement {
    pub label: String,
    pub ok: bool,
    pub wall_secs: f64,
    /// Pico de memória residente (KiB), se medível.
    pub max_rss_kb: Option<u64>,
    /// Tamanho do binário (bytes), se conhecido.
    pub binary_bytes: Option<u64>,
}

/// GNU `time -v` está disponível?
pub fn gnu_time_available() -> bool {
    std::path::Path::new("/usr/bin/time").exists()
}

/// Corre `program args` (com env extra), medindo tempo e, se possível, RSS máx.
/// Herda stdout/stderr do filho para stderr do pai não poluir a saída útil.
pub fn run_measured(
    label: &str,
    program: &str,
    args: &[String],
    envs: &[(String, String)],
    cwd: Option<&Utf8Path>,
) -> std::io::Result<Measurement> {
    let use_time = gnu_time_available();
    let rss_file = if use_time {
        Some(tempfile::NamedTempFile::new()?)
    } else {
        None
    };

    let mut cmd;
    if let Some(f) = &rss_file {
        cmd = Command::new("/usr/bin/time");
        cmd.args(["-v", "-o"]).arg(f.path()).arg(program).args(args);
    } else {
        cmd = Command::new(program);
        cmd.args(args);
    }
    for (k, v) in envs {
        cmd.env(k, v);
    }
    if let Some(c) = cwd {
        cmd.current_dir(c);
    }

    let start = Instant::now();
    let status = cmd.status()?;
    let wall = start.elapsed().as_secs_f64();

    let max_rss_kb = rss_file
        .as_ref()
        .and_then(|f| std::fs::read_to_string(f.path()).ok())
        .and_then(|s| parse_max_rss(s.as_str()));

    Ok(Measurement {
        label: label.to_string(),
        ok: status.success(),
        wall_secs: wall,
        max_rss_kb,
        binary_bytes: None,
    })
}

/// Extrai "Maximum resident set size (kbytes): N" da saída do GNU time.
fn parse_max_rss(s: &str) -> Option<u64> {
    for line in s.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("Maximum resident set size (kbytes):") {
            return rest.trim().parse::<u64>().ok();
        }
    }
    None
}

/// Tamanho em bytes de um ficheiro (ex.: binário produzido).
pub fn file_size(path: &Utf8Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gnu_time_rss() {
        let sample = "\tCommand being timed: \"true\"\n\tMaximum resident set size (kbytes): 1132\n\tExit status: 0\n";
        assert_eq!(parse_max_rss(sample), Some(1132));
    }

    #[test]
    fn missing_rss_is_none() {
        assert_eq!(parse_max_rss("sem nada relevante"), None);
    }

    #[test]
    fn measures_a_trivial_command() {
        let m = run_measured("true", "true", &[], &[], None).unwrap();
        assert!(m.ok);
        assert!(m.wall_secs >= 0.0);
        // max_rss pode ser None se não houver GNU time; não forçamos.
    }
}
