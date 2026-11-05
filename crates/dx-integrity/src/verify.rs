//! Verificação de checksum de artefactos `.crate` em cache.

use camino::{Utf8Path, Utf8PathBuf};
use sha2::{Digest, Sha256};
use std::io::Read;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IntegrityError {
    #[error("checksum mismatch for {name}@{version}: expected {expected}, got {actual}")]
    Mismatch {
        name: String,
        version: String,
        expected: String,
        actual: String,
    },
    #[error("erro de I/O ao verificar {0}: {1}")]
    Io(String, String),
}

/// sha256 (hex) de um ficheiro, lido em blocos (não carrega tudo em memória).
pub fn sha256_file(path: &Utf8Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Raiz do cache de registry do Cargo (`$CARGO_HOME/registry/cache`).
pub fn registry_cache_root() -> Utf8PathBuf {
    let home = std::env::var("CARGO_HOME")
        .ok()
        .map(Utf8PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| Utf8PathBuf::from(h).join(".cargo"))
        })
        .unwrap_or_else(|| Utf8PathBuf::from(".cargo"));
    home.join("registry").join("cache")
}

/// Localiza o `.crate` em cache para `name-version` (em qualquer subdir de
/// índice). `None` se não estiver descarregado.
pub fn cached_crate_path(name: &str, version: &str) -> Option<Utf8PathBuf> {
    let root = registry_cache_root();
    let file_name = format!("{name}-{version}.crate");
    let entries = std::fs::read_dir(root.as_std_path()).ok()?;
    for e in entries.flatten() {
        let candidate = Utf8PathBuf::from_path_buf(e.path().join(&file_name)).ok()?;
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Verifica que o `.crate` em `path` tem o `expected` sha256.
pub fn verify_crate(
    path: &Utf8Path,
    name: &str,
    version: &str,
    expected: &str,
) -> Result<(), IntegrityError> {
    let actual =
        sha256_file(path).map_err(|e| IntegrityError::Io(path.to_string(), e.to_string()))?;
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(IntegrityError::Mismatch {
            name: name.to_string(),
            version: version.to_string(),
            expected: expected.to_string(),
            actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_tampering() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        let p = dir.join("x.crate");
        std::fs::write(&p, b"conteudo original").unwrap();
        let good = sha256_file(&p).unwrap();
        // Verifica com o checksum correcto.
        assert!(verify_crate(&p, "x", "1.0.0", &good).is_ok());
        // Adultera os bytes → mismatch.
        std::fs::write(&p, b"conteudo ADULTERADO").unwrap();
        let err = verify_crate(&p, "x", "1.0.0", &good).unwrap_err();
        assert!(matches!(err, IntegrityError::Mismatch { .. }));
    }
}
