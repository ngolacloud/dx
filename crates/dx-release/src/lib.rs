//! `dx-release` — preparação de release. **Prepara ≠ publica**: produz
//! artefactos para revisão (binários+checksums, SBOM, proveniência, ordem de
//! publicação), sem criar tags, mudar versões ou publicar (contrato §8).
//!
//! É idempotente: repetir a preparação reescreve os mesmos ficheiros em
//! `target/dx-release/`, sem duplicar nada.

pub mod sbom;
pub mod topo;

pub use topo::{members_from_metadata, publish_order, Member};

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use serde_json::Value;

pub use dx_integrity::sha256_file;

/// Um artefacto produzido, com a sua proveniência de integridade.
#[derive(Debug, Clone, Serialize)]
pub struct Artifact {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

/// Metadados de proveniência da preparação.
#[derive(Debug, Clone, Serialize)]
pub struct Provenance {
    pub tool: String,
    /// Timestamp ISO-ish (fornecido pelo chamador).
    pub generated: String,
    pub rustc: String,
    pub host_target: String,
    pub git_commit: Option<String>,
    pub artifacts: Vec<Artifact>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReleaseError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization: {0}")]
    Json(#[from] serde_json::Error),
}

/// Directório de staging dos artefactos (dentro de `target/`, gitignorável).
pub fn release_dir(root: &Utf8Path) -> Utf8PathBuf {
    root.join("target").join("dx-release")
}

/// Calcula o `Artifact` de um ficheiro (sha256 + tamanho).
pub fn artifact_of(path: &Utf8Path) -> std::io::Result<Artifact> {
    let bytes = std::fs::metadata(path)?.len();
    let sha256 = sha256_file(path)?;
    Ok(Artifact {
        path: path.to_string(),
        sha256,
        bytes,
    })
}

/// Escreve os artefactos de revisão em `target/dx-release/` (idempotente).
/// Devolve o caminho do directório.
pub fn write_review_bundle(
    root: &Utf8Path,
    sbom: &Value,
    provenance: &Provenance,
    publish_order: &[String],
) -> Result<Utf8PathBuf, ReleaseError> {
    let dir = release_dir(root);
    std::fs::create_dir_all(dir.as_std_path())?;

    std::fs::write(
        dir.join("sbom.cdx.json").as_std_path(),
        serde_json::to_vec_pretty(sbom)?,
    )?;
    std::fs::write(
        dir.join("provenance.json").as_std_path(),
        serde_json::to_vec_pretty(provenance)?,
    )?;
    std::fs::write(
        dir.join("publish-order.txt").as_std_path(),
        publish_order.join("\n") + "\n",
    )?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_and_bundle_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(tmp.path()).unwrap();
        let f = root.join("bin");
        std::fs::write(f.as_std_path(), b"conteudo").unwrap();
        let a = artifact_of(&f).unwrap();
        assert_eq!(a.bytes, 8);
        assert_eq!(a.sha256.len(), 64);

        let prov = Provenance {
            tool: "dx".into(),
            generated: "2026-11-05T00:00:00Z".into(),
            rustc: "rustc x".into(),
            host_target: "x86_64".into(),
            git_commit: None,
            artifacts: vec![a],
        };
        let bom = serde_json::json!({"bomFormat":"CycloneDX"});
        let dir =
            write_review_bundle(root, &bom, &prov, &["dx-core".into(), "dx-cli".into()]).unwrap();
        assert!(dir.join("sbom.cdx.json").exists());
        assert!(dir.join("provenance.json").exists());
        let order = std::fs::read_to_string(dir.join("publish-order.txt").as_std_path()).unwrap();
        assert_eq!(order, "dx-core\ndx-cli\n");
    }
}
