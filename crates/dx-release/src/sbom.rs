//! SBOM em CycloneDX (mínimo mas real) a partir do Cargo.lock.
//!
//! Cada package resolvido vira um componente com `purl` (pkg:cargo/...) e, para
//! registry, o hash sha256 fixado no lockfile. Isto é proveniência verificável
//! da cadeia (contrato §8), não uma garantia de benignidade.

use camino::Utf8Path;
use dx_integrity::{all_packages, PackageEntry};
use serde_json::{json, Value};

#[derive(Debug, thiserror::Error)]
pub enum SbomError {
    #[error("ler lockfile: {0}")]
    Lock(#[from] dx_integrity::lockfile::LockError),
}

/// Constrói um documento CycloneDX 1.5 (JSON) com os componentes do lockfile.
/// `generated` é um timestamp ISO-ish fornecido pelo chamador (não geramos
/// tempo aqui para manter a função determinística/testável).
pub fn cyclonedx(lockfile: &Utf8Path, generated: &str) -> Result<Value, SbomError> {
    let pkgs = all_packages(lockfile)?;
    let components: Vec<Value> = pkgs.iter().map(component).collect();
    Ok(json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "metadata": { "timestamp": generated, "tools": [{ "name": "dx", "vendor": "NgolaCloud" }] },
        "components": components,
    }))
}

fn component(p: &PackageEntry) -> Value {
    let mut c = json!({
        "type": "library",
        "name": p.name,
        "version": p.version,
        "purl": format!("pkg:cargo/{}@{}", p.name, p.version),
    });
    if let Some(cs) = &p.checksum {
        c["hashes"] = json!([{ "alg": "SHA-256", "content": cs }]);
    }
    if let Some(src) = &p.source {
        c["properties"] = json!([{ "name": "cargo:source", "value": src }]);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_cyclonedx_from_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8Path::from_path(tmp.path()).unwrap();
        let lock = dir.join("Cargo.lock");
        std::fs::write(
            &lock,
            "version = 4\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nsource = \"registry+https://x\"\nchecksum = \"deadbeef\"\n",
        )
        .unwrap();
        let bom = cyclonedx(&lock, "2026-11-05T00:00:00Z").unwrap();
        assert_eq!(bom["bomFormat"], "CycloneDX");
        let comp = &bom["components"][0];
        assert_eq!(comp["purl"], "pkg:cargo/serde@1.0.0");
        assert_eq!(comp["hashes"][0]["content"], "deadbeef");
    }
}
