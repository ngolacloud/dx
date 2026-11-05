//! Extracção segura de arquivos `.crate` (tar.gz).
//!
//! Um `.crate` é um tarball gzip. Ao desempacotar, um arquivo hostil pode tentar
//! (contrato §5, "Extracção segura"):
//! - escapar do destino por caminho absoluto ou `..` (path traversal);
//! - escapar por symlink/hardlink que aponta para fora;
//! - esgotar o disco por descompressão descontrolada (zip-bomb) ou por nº
//!   excessivo de entradas.
//!
//! Esta função bloqueia tudo isso **antes** de escrever cada entrada. É
//! conservadora: recusa symlinks e hardlinks por completo (os `.crate` normais
//! não os usam). Limite honesto: não protege contra um atacante com controlo
//! total do mesmo utilizador/sistema (ver threat model).

use camino::{Utf8Path, Utf8PathBuf};
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path};

/// Limites de recursos da extracção (defesa anti zip-bomb).
#[derive(Debug, Clone)]
pub struct Limits {
    pub max_entries: usize,
    pub max_total_bytes: u64,
    pub max_file_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_entries: 20_000,
            max_total_bytes: 256 * 1024 * 1024, // 256 MiB
            max_file_bytes: 64 * 1024 * 1024,   // 64 MiB
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExtractError {
    #[error("entry with absolute path rejected: {0}")]
    AbsolutePath(String),
    #[error("entry with `..` (path traversal) rejected: {0}")]
    PathTraversal(String),
    #[error("symlink rejected (conservative extraction): {0}")]
    SymlinkRejected(String),
    #[error("hardlink rejected (conservative extraction): {0}")]
    HardlinkRejected(String),
    #[error("unsupported entry type rejected: {0}")]
    UnsupportedEntry(String),
    #[error("too many entries (> {0})")]
    TooManyEntries(usize),
    #[error("total size exceeds limit (> {0} bytes)")]
    TotalSizeExceeded(u64),
    #[error("file exceeds per-file limit (> {0} bytes): {1}")]
    FileSizeExceeded(u64, String),
    #[error("I/O/archive error: {0}")]
    Io(String),
}

impl From<io::Error> for ExtractError {
    fn from(e: io::Error) -> Self {
        ExtractError::Io(e.to_string())
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ExtractReport {
    pub files: usize,
    pub dirs: usize,
    pub total_bytes: u64,
}

/// Valida um caminho de entrada e devolve o caminho relativo seguro.
/// Recusa caminhos absolutos e qualquer componente `..`.
fn safe_relative(path: &Path) -> Result<Utf8PathBuf, ExtractError> {
    let shown = path.display().to_string();
    let mut rel = Utf8PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Normal(p) => {
                let s = p
                    .to_str()
                    .ok_or_else(|| ExtractError::Io("non-UTF8 name".into()))?;
                rel.push(s);
            }
            Component::CurDir => {}
            Component::ParentDir => return Err(ExtractError::PathTraversal(shown)),
            Component::RootDir | Component::Prefix(_) => {
                return Err(ExtractError::AbsolutePath(shown))
            }
        }
    }
    Ok(rel)
}

/// Desempacota `archive` (tar.gz) para `dest` com todas as salvaguardas.
/// `dest` é criado se não existir. Não executa qualquer código do arquivo.
pub fn safe_unpack(
    archive_path: &Utf8Path,
    dest: &Utf8Path,
    limits: &Limits,
) -> Result<ExtractReport, ExtractError> {
    let file = fs::File::open(archive_path)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    fs::create_dir_all(dest)?;

    let mut report = ExtractReport::default();

    for entry in tar.entries()? {
        let mut entry = entry?;
        if report.files + report.dirs + 1 > limits.max_entries {
            return Err(ExtractError::TooManyEntries(limits.max_entries));
        }

        let etype = entry.header().entry_type();
        let path = entry.path().map_err(|e| ExtractError::Io(e.to_string()))?;
        let shown = path.display().to_string();

        // Rejeitar links antes de qualquer escrita.
        if etype.is_symlink() {
            return Err(ExtractError::SymlinkRejected(shown));
        }
        if etype.is_hard_link() {
            return Err(ExtractError::HardlinkRejected(shown));
        }

        let rel = safe_relative(&path)?;
        let out_path = dest.join(&rel);

        if etype.is_dir() {
            fs::create_dir_all(&out_path)?;
            report.dirs += 1;
            continue;
        }
        if !etype.is_file() {
            return Err(ExtractError::UnsupportedEntry(format!(
                "{shown} ({etype:?})"
            )));
        }

        // Limite por-ficheiro declarado no header (sinal rápido).
        let declared = entry.header().size().unwrap_or(0);
        if declared > limits.max_file_bytes {
            return Err(ExtractError::FileSizeExceeded(limits.max_file_bytes, shown));
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Copiar limitando os bytes REAIS lidos (defesa contra header mentiroso
        // e contra descompressão descontrolada).
        let remaining_total = limits.max_total_bytes.saturating_sub(report.total_bytes);
        let cap = limits.max_file_bytes.min(remaining_total);
        let mut limited = entry.by_ref().take(cap + 1);
        let mut out = fs::File::create(&out_path)?;
        let written = io::copy(&mut limited, &mut out)?;
        if written > cap {
            // Excedeu: apagar o parcial e falhar com a causa correcta.
            drop(out);
            let _ = fs::remove_file(&out_path);
            if report.total_bytes + written > limits.max_total_bytes {
                return Err(ExtractError::TotalSizeExceeded(limits.max_total_bytes));
            }
            return Err(ExtractError::FileSizeExceeded(limits.max_file_bytes, shown));
        }
        report.total_bytes += written;
        report.files += 1;
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Constrói um tar.gz em memória a partir de (header, path, conteúdo).
    fn make_targz(
        build: impl FnOnce(&mut tar::Builder<flate2::write::GzEncoder<Vec<u8>>>),
    ) -> Vec<u8> {
        let enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut b = tar::Builder::new(enc);
        build(&mut b);
        let enc = b.into_inner().unwrap();
        enc.finish().unwrap()
    }

    fn write_archive(bytes: &[u8], dir: &Utf8Path) -> Utf8PathBuf {
        let p = dir.join("a.crate");
        fs::write(&p, bytes).unwrap();
        p
    }

    fn file_entry(
        b: &mut tar::Builder<flate2::write::GzEncoder<Vec<u8>>>,
        path: &str,
        data: &[u8],
    ) {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_entry_type(tar::EntryType::Regular);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, path, data).unwrap();
    }

    #[test]
    fn extracts_a_normal_archive() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        let bytes = make_targz(|b| {
            file_entry(b, "pkg-1.0/src/lib.rs", b"fn x() {}");
            file_entry(b, "pkg-1.0/Cargo.toml", b"[package]");
        });
        let arc = write_archive(&bytes, dir);
        let out = dir.join("out");
        let rep = safe_unpack(&arc, &out, &Limits::default()).unwrap();
        assert_eq!(rep.files, 2);
        assert!(out.join("pkg-1.0/src/lib.rs").exists());
    }

    #[test]
    fn rejects_path_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        // O builder normal recusa `..`; escrevemos o nome directamente nos
        // bytes do header para simular um arquivo hostil (bypass à validação).
        let bytes = make_targz(|b| {
            let data = b"evil";
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_entry_type(tar::EntryType::Regular);
            h.set_mode(0o644);
            let name = b"../escape.rs";
            h.as_mut_bytes()[..name.len()].copy_from_slice(name);
            h.set_cksum();
            b.append(&h, &data[..]).unwrap();
        });
        let arc = write_archive(&bytes, dir);
        let out = dir.join("out");
        let err = safe_unpack(&arc, &out, &Limits::default()).unwrap_err();
        assert!(matches!(err, ExtractError::PathTraversal(_)), "{err:?}");
        // Nada escapou.
        assert!(!dir.join("escape.rs").exists());
    }

    #[test]
    fn rejects_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        let bytes = make_targz(|b| {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            h.set_mode(0o777);
            b.append_link(&mut h, "pkg/link", "/etc/passwd").unwrap();
        });
        let arc = write_archive(&bytes, dir);
        let out = dir.join("out");
        let err = safe_unpack(&arc, &out, &Limits::default()).unwrap_err();
        assert!(matches!(err, ExtractError::SymlinkRejected(_)), "{err:?}");
    }

    #[test]
    fn rejects_decompression_bomb_by_total_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        // Dois ficheiros de 2 KiB cada; limite total de 3 KiB ⇒ estoura.
        let big = vec![0u8; 2048];
        let bytes = make_targz(|b| {
            file_entry(b, "pkg/a.bin", &big);
            file_entry(b, "pkg/b.bin", &big);
        });
        let arc = write_archive(&bytes, dir);
        let out = dir.join("out");
        let limits = Limits {
            max_entries: 100,
            max_total_bytes: 3 * 1024,
            max_file_bytes: 64 * 1024,
        };
        let err = safe_unpack(&arc, &out, &limits).unwrap_err();
        assert!(matches!(err, ExtractError::TotalSizeExceeded(_)), "{err:?}");
    }

    #[test]
    fn rejects_oversized_single_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        let big = vec![7u8; 4096];
        let bytes = make_targz(|b| file_entry(b, "pkg/huge.bin", &big));
        let arc = write_archive(&bytes, dir);
        let out = dir.join("out");
        let limits = Limits {
            max_entries: 100,
            max_total_bytes: 1024 * 1024,
            max_file_bytes: 1024, // 1 KiB
        };
        let err = safe_unpack(&arc, &out, &limits).unwrap_err();
        assert!(
            matches!(err, ExtractError::FileSizeExceeded(_, _)),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_too_many_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        let bytes = make_targz(|b| {
            for i in 0..10 {
                file_entry(b, &format!("pkg/f{i}.txt"), b"x");
            }
        });
        let arc = write_archive(&bytes, dir);
        let out = dir.join("out");
        let limits = Limits {
            max_entries: 5,
            ..Limits::default()
        };
        let err = safe_unpack(&arc, &out, &limits).unwrap_err();
        assert!(matches!(err, ExtractError::TooManyEntries(5)), "{err:?}");
    }

    #[test]
    fn safe_relative_rejects_absolute() {
        assert!(matches!(
            safe_relative(Path::new("/etc/passwd")),
            Err(ExtractError::AbsolutePath(_))
        ));
    }
}
