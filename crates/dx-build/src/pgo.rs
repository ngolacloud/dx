//! PGO (Profile-Guided Optimization): instrumentar → correr workload → fundir
//! perfis com `llvm-profdata` → recompilar com o perfil → comparar.
//!
//! Contrato §7: os perfis têm de corresponder ao código/toolchain/target; um
//! perfil incompatível ou ausente é **reportado**, não ignorado. A versão do
//! `llvm-profdata` tem de bater com o LLVM do rustc (senão a fusão falha).

use camino::{Utf8Path, Utf8PathBuf};
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum PgoError {
    #[error("llvm-profdata not found (install `rustup component add llvm-tools-preview` or have /usr/bin/llvm-profdata)")]
    NoProfdata,
    #[error(
        "no .profraw file in {0} — did the workload run and exercise the instrumented binary?"
    )]
    NoProfraw(String),
    #[error("llvm-profdata merge failed (profile incompatible with the toolchain?): {0}")]
    MergeFailed(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Localiza o `llvm-profdata`: primeiro na sysroot do rustc (rustup
/// llvm-tools), depois no PATH do sistema (`/usr/bin`).
pub fn profdata_tool() -> Option<Utf8PathBuf> {
    // sysroot/lib/rustlib/<host>/bin/llvm-profdata
    if let Ok(out) = Command::new("rustc").args(["--print", "sysroot"]).output() {
        if out.status.success() {
            let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_string();
            // procurar em qualquer target dir
            let base = Utf8PathBuf::from(sysroot).join("lib/rustlib");
            if let Ok(entries) = std::fs::read_dir(base.as_std_path()) {
                for e in entries.flatten() {
                    let cand = e.path().join("bin").join("llvm-profdata");
                    if cand.exists() {
                        if let Ok(p) = Utf8PathBuf::from_path_buf(cand) {
                            return Some(p);
                        }
                    }
                }
            }
        }
    }
    let system = Utf8PathBuf::from("/usr/bin/llvm-profdata");
    if system.exists() {
        return Some(system);
    }
    None
}

/// RUSTFLAGS para a fase de instrumentação.
pub fn rustflags_generate(raw_dir: &Utf8Path) -> String {
    format!("-Cprofile-generate={raw_dir}")
}

/// RUSTFLAGS para a fase de uso do perfil. Avisa (não falha) em funções sem
/// dados de perfil, úteis para detectar perfis desalinhados do código.
pub fn rustflags_use(merged: &Utf8Path) -> String {
    format!("-Cprofile-use={merged} -Cllvm-args=-pgo-warn-missing-function")
}

/// Funde todos os `.profraw` de `raw_dir` num `.profdata`.
pub fn merge(raw_dir: &Utf8Path, out: &Utf8Path) -> Result<usize, PgoError> {
    let tool = profdata_tool().ok_or(PgoError::NoProfdata)?;
    let mut raws = Vec::new();
    for e in std::fs::read_dir(raw_dir.as_std_path())?.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("profraw") {
            raws.push(p);
        }
    }
    if raws.is_empty() {
        return Err(PgoError::NoProfraw(raw_dir.to_string()));
    }
    let n = raws.len();
    let mut cmd = Command::new(tool.as_str());
    cmd.arg("merge").arg("-o").arg(out.as_str());
    for r in &raws {
        cmd.arg(r);
    }
    let output = cmd.output()?;
    if !output.status.success() {
        return Err(PgoError::MergeFailed(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rustflags_are_well_formed() {
        let g = rustflags_generate(Utf8Path::new("/tmp/raw"));
        assert!(g.contains("-Cprofile-generate=/tmp/raw"));
        let u = rustflags_use(Utf8Path::new("/tmp/m.profdata"));
        assert!(u.contains("-Cprofile-use=/tmp/m.profdata"));
        assert!(u.contains("pgo-warn-missing-function"));
    }

    #[test]
    fn merge_without_profraw_reports_clearly() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        // Só testamos o caminho "sem profraw" se a ferramenta existir; senão
        // o erro é NoProfdata (também aceitável).
        match merge(dir, &dir.join("out.profdata")) {
            Err(PgoError::NoProfraw(_)) | Err(PgoError::NoProfdata) => {}
            other => panic!("esperava NoProfraw/NoProfdata, veio {other:?}"),
        }
    }
}
