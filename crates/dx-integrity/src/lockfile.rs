//! Leitura dos checksums do Cargo.lock (metadados confiáveis de integridade).
//!
//! O Cargo.lock fixa, por package de registry, o `checksum` sha256 do `.crate`.
//! É contra isto que verificamos os bytes em cache. Checksum prova integridade
//! **em relação à origem fixada**, não benignidade (contrato §5.4).

use camino::Utf8Path;
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("could not read {0}")]
    Read(String),
    #[error("invalid Cargo.lock: {0}")]
    Parse(String),
}

/// Um package de registry com checksum fixado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageChecksum {
    pub name: String,
    pub version: String,
    pub checksum: String,
    pub source: String,
}

#[derive(Deserialize)]
struct Lock {
    #[serde(default)]
    package: Vec<LockPkg>,
}

#[derive(Deserialize)]
struct LockPkg {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
}

/// Extrai os packages de registry com checksum do Cargo.lock.
/// Packages git/path (sem checksum de registry) ficam de fora — a sua
/// integridade é a revisão/fingerprint, tratada à parte (ver verify.rs).
pub fn registry_checksums(lockfile: &Utf8Path) -> Result<Vec<PackageChecksum>, LockError> {
    Ok(all_packages(lockfile)?
        .into_iter()
        .filter_map(|p| {
            let source = p.source?;
            let checksum = p.checksum?;
            if source.starts_with("registry+") {
                Some(PackageChecksum {
                    name: p.name,
                    version: p.version,
                    checksum,
                    source,
                })
            } else {
                None
            }
        })
        .collect())
}

/// Uma entrada qualquer do Cargo.lock (registry, git, path ou local).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageEntry {
    pub name: String,
    pub version: String,
    pub source: Option<String>,
    pub checksum: Option<String>,
}

/// Packages presentes em `after` mas não em `before` (por nome+versão).
/// Base do "o que estou a introduzir?" do `dx add --plan`.
pub fn diff_added(before: &[PackageEntry], after: &[PackageEntry]) -> Vec<PackageEntry> {
    use std::collections::BTreeSet;
    let seen: BTreeSet<(&str, &str)> = before
        .iter()
        .map(|p| (p.name.as_str(), p.version.as_str()))
        .collect();
    let mut out: Vec<PackageEntry> = after
        .iter()
        .filter(|p| !seen.contains(&(p.name.as_str(), p.version.as_str())))
        .cloned()
        .collect();
    out.sort_by(|a, b| {
        (a.name.as_str(), a.version.as_str()).cmp(&(b.name.as_str(), b.version.as_str()))
    });
    out
}

/// Todos os packages do Cargo.lock (para SBOM/proveniência).
pub fn all_packages(lockfile: &Utf8Path) -> Result<Vec<PackageEntry>, LockError> {
    let text = std::fs::read_to_string(lockfile).map_err(|e| LockError::Read(e.to_string()))?;
    let lock: Lock = toml::from_str(&text).map_err(|e| LockError::Parse(e.to_string()))?;
    Ok(lock
        .package
        .into_iter()
        .map(|p| PackageEntry {
            name: p.name,
            version: p.version,
            source: p.source,
            checksum: p.checksum,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_registry_packages_only() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let lock = dir.join("Cargo.lock");
        std::fs::write(
            &lock,
            r#"
version = 4

[[package]]
name = "demo"
version = "0.1.0"

[[package]]
name = "serde"
version = "1.0.229"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "abc123"

[[package]]
name = "localdep"
version = "0.1.0"
source = "git+https://example/x#deadbeef"
"#,
        )
        .unwrap();
        let cs = registry_checksums(&lock).unwrap();
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].name, "serde");
        assert_eq!(cs[0].checksum, "abc123");
    }

    #[test]
    fn diff_added_finds_new_packages() {
        let mk = |n: &str, v: &str| PackageEntry {
            name: n.into(),
            version: v.into(),
            source: None,
            checksum: None,
        };
        let before = vec![mk("serde", "1.0.0"), mk("libc", "0.2.0")];
        let after = vec![
            mk("serde", "1.0.0"),
            mk("libc", "0.2.0"),
            mk("anyhow", "1.0.0"),
            mk("serde", "1.1.0"), // nova versão também conta
        ];
        let added = diff_added(&before, &after);
        assert_eq!(added.len(), 2);
        assert!(added.iter().any(|p| p.name == "anyhow"));
        assert!(added
            .iter()
            .any(|p| p.name == "serde" && p.version == "1.1.0"));
    }
}
