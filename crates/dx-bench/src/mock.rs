//! Gerador de **mock registry** determinista: N crates locais (por `path`) que
//! o root consome. Dá um grafo de tamanho controlado sem rede nem registry real
//! — para escala reproduzível (contrato §9). Amostra real controlada fica
//! documentada como não implementada (ver docs/benchmarks.md).

use camino::Utf8Path;
use std::io;

/// Gera um workspace-mock em `root` com `n` dependências locais.
/// Determinístico: mesmos nomes/conteúdos para o mesmo `n`.
pub fn gen_mock_workspace(root: &Utf8Path, n: usize) -> io::Result<()> {
    std::fs::create_dir_all(root.join("src").as_std_path())?;
    std::fs::create_dir_all(root.join("crates").as_std_path())?;

    // Root manifest com n dependências path.
    let mut deps = String::new();
    for i in 0..n {
        let name = dep_name(i);
        deps.push_str(&format!("{name} = {{ path = \"crates/{name}\" }}\n"));
    }
    let root_manifest = format!(
        "[package]\nname = \"bench-root\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[dependencies]\n{deps}"
    );
    std::fs::write(root.join("Cargo.toml").as_std_path(), root_manifest)?;
    std::fs::write(root.join("src/lib.rs").as_std_path(), b"")?;

    // Cada dependência: crate lib mínimo, determinístico.
    for i in 0..n {
        let name = dep_name(i);
        let dir = root.join("crates").join(&name);
        std::fs::create_dir_all(dir.join("src").as_std_path())?;
        let manifest = format!(
            "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[dependencies]\n"
        );
        std::fs::write(dir.join("Cargo.toml").as_std_path(), manifest)?;
        std::fs::write(
            dir.join("src/lib.rs").as_std_path(),
            format!("pub fn id() -> u32 {{ {i} }}\n"),
        )?;
    }
    Ok(())
}

fn dep_name(i: usize) -> String {
    format!("dxdep{i:04}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_deterministic_graph() {
        let tmp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(tmp.path()).unwrap();
        gen_mock_workspace(root, 5).unwrap();
        assert!(root.join("Cargo.toml").exists());
        let manifest = std::fs::read_to_string(root.join("Cargo.toml").as_std_path()).unwrap();
        assert!(manifest.contains("dxdep0000 = { path = \"crates/dxdep0000\" }"));
        assert!(manifest.contains("dxdep0004"));
        assert!(root.join("crates/dxdep0004/src/lib.rs").exists());
        // Determinístico: regenerar dá o mesmo manifesto.
        let tmp2 = tempfile::tempdir().unwrap();
        let root2 = Utf8Path::from_path(tmp2.path()).unwrap();
        gen_mock_workspace(root2, 5).unwrap();
        let manifest2 = std::fs::read_to_string(root2.join("Cargo.toml").as_std_path()).unwrap();
        assert_eq!(manifest, manifest2);
    }
}
