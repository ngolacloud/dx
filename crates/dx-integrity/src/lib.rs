//! `dx-integrity` — integridade de artefactos e extracção segura.
//!
//! Dois papéis distintos (contrato §5):
//! - **Integridade vs origem**: verificar o sha256 dos `.crate` em cache contra
//!   o checksum fixado no Cargo.lock. Prova que os bytes consumidos são os que
//!   a resolução fixou — **não** prova benignidade.
//! - **Extracção segura**: desempacotar `.crate` bloqueando path traversal,
//!   symlinks/hardlinks e descompressão descontrolada (zip-bomb).
//!
//! Limite honesto: não protege contra um atacante com controlo total do mesmo
//! utilizador/sistema (ver threat model).

pub mod extract;
pub mod lockfile;
pub mod verify;

pub use extract::{safe_unpack, ExtractError, ExtractReport, Limits};
pub use lockfile::{registry_checksums, PackageChecksum};
pub use verify::{cached_crate_path, sha256_file, verify_crate, IntegrityError};

use camino::Utf8Path;

/// Resultado da verificação de integridade de um lockfile.
#[derive(Debug, Default)]
pub struct IntegrityReport {
    /// Packages cujo `.crate` em cache bate com o checksum do lockfile.
    pub verified: Vec<String>,
    /// Packages de registry sem `.crate` em cache (ainda não descarregados):
    /// **sem cobertura**, nunca "comprovadamente íntegros".
    pub missing: Vec<String>,
    /// Packages cujo `.crate` em cache **não** bate — bloqueio duro.
    pub mismatched: Vec<IntegrityError>,
    /// Packages cuja extracção segura falhou (só se `extract_check`).
    pub extract_failures: Vec<(String, ExtractError)>,
}

impl IntegrityReport {
    /// Há falha de integridade dura (mismatch ou extracção insegura)?
    pub fn has_failure(&self) -> bool {
        !self.mismatched.is_empty() || !self.extract_failures.is_empty()
    }
}

/// Verifica a integridade de todos os packages de registry de um lockfile
/// contra os `.crate` em cache. Se `extract_check`, também desempacota cada
/// `.crate` verificado para um temporário com as salvaguardas, confirmando que
/// extrai dentro dos limites e sem escapar — e descarta.
pub fn verify_lockfile(
    lockfile: &Utf8Path,
    limits: &Limits,
    extract_check: bool,
) -> Result<IntegrityReport, lockfile::LockError> {
    let pkgs = registry_checksums(lockfile)?;
    let mut report = IntegrityReport::default();

    for p in pkgs {
        let label = format!("{}@{}", p.name, p.version);
        let path = match cached_crate_path(&p.name, &p.version) {
            Some(path) => path,
            None => {
                report.missing.push(label);
                continue;
            }
        };
        match verify_crate(&path, &p.name, &p.version, &p.checksum) {
            Ok(()) => {
                if extract_check {
                    if let Err(e) = extract_check_one(&path, limits) {
                        report.extract_failures.push((label.clone(), e));
                    }
                }
                report.verified.push(label);
            }
            Err(e) => report.mismatched.push(e),
        }
    }
    Ok(report)
}

/// Extrai para um temporário com salvaguardas e descarta (dry-run de extracção).
fn extract_check_one(crate_path: &Utf8Path, limits: &Limits) -> Result<(), ExtractError> {
    let tmp = tempfile::tempdir().map_err(|e| ExtractError::Io(e.to_string()))?;
    let dest =
        Utf8Path::from_path(tmp.path()).ok_or_else(|| ExtractError::Io("temp não-UTF8".into()))?;
    safe_unpack(crate_path, dest, limits).map(|_| ())
}
