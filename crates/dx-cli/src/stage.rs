//! Resolução do grafo candidato num **staging isolado** (contrato §5.1).
//!
//! Para decidir se uma nova dependência pode aterrar, precisamos de resolver o
//! grafo que ela produziria — mas **sem** compilar nem executar `build.rs`,
//! proc-macros, testes, exemplos ou hooks. `cargo generate-lockfile` faz
//! exactamente isso: resolve a partir do índice e escreve um Cargo.lock sem
//! correr qualquer código das dependências.
//!
//! O staging é uma cópia isolada do manifesto mutado + lockfile existente, num
//! directório temporário. Assim, o manifesto real nunca é tocado antes da
//! decisão: bytes/lockfile podem ser produzidos no staging, mas nada aterra no
//! projecto sem aprovação.
//!
//! Limite honesto: esta versão cobre manifestos **auto-contidos** (o que o
//! `dx new` gera). Projectos com `[workspace]`/herança de workspace ou
//! dependências `path` caem em `NotApplicable` (a cópia isolada quebraria a
//! resolução), e o `add` reporta estado não-verificado em vez de fingir gate.

use std::process::Command;

use camino::{Utf8Path, Utf8PathBuf};
use tempfile::TempDir;

pub enum Staging {
    /// Grafo candidato resolvido; `root` contém Cargo.toml + Cargo.lock.
    Resolved { _guard: TempDir, root: Utf8PathBuf },
    /// Forma de projecto fora do âmbito desta versão do staging.
    NotApplicable { reason: String },
    /// Resolução falhou (ex.: offline sem cache, versão inexistente).
    ResolveFailed { reason: String },
}

/// Heurística de auto-contenção do manifesto mutado.
fn self_contained(toml: &str) -> Result<(), String> {
    if toml.contains("[workspace]") || toml.contains("workspace.dependencies") {
        return Err("workspace manifest".into());
    }
    if toml.contains(".workspace = true") || toml.contains("workspace = true") {
        return Err("uses workspace inheritance (`.workspace = true`)".into());
    }
    if toml.contains("path = ") || toml.contains("path.workspace") {
        return Err("has `path` dependencies (relative paths break in staging)".into());
    }
    Ok(())
}

/// Monta o staging e resolve o grafo candidato.
/// `mutated_toml` é o Cargo.toml já com a dependência pretendida.
pub fn resolve_candidate(project_root: &Utf8Path, mutated_toml: &str, offline: bool) -> Staging {
    if let Err(reason) = self_contained(mutated_toml) {
        return Staging::NotApplicable { reason };
    }

    let dir = match TempDir::new() {
        Ok(d) => d,
        Err(e) => {
            return Staging::ResolveFailed {
                reason: format!("could not create staging: {e}"),
            }
        }
    };
    let root = match Utf8Path::from_path(dir.path()) {
        Some(p) => p.to_owned(),
        None => {
            return Staging::ResolveFailed {
                reason: "non-UTF8 staging path".into(),
            }
        }
    };

    if let Err(e) = std::fs::write(root.join("Cargo.toml"), mutated_toml) {
        return Staging::ResolveFailed {
            reason: format!("write staging manifest: {e}"),
        };
    }
    // Um src/ mínimo para o Cargo aceitar o manifesto ao resolver.
    let _ = std::fs::create_dir_all(root.join("src"));
    let _ = std::fs::write(root.join("src/lib.rs"), "");

    // Reaproveitar o lockfile existente como ponto de partida da resolução
    // (minimiza churn e evita mudar versões já fixadas).
    let existing_lock = project_root.join("Cargo.lock");
    if existing_lock.exists() {
        let _ = std::fs::copy(&existing_lock, root.join("Cargo.lock"));
    }

    // Resolver SEM compilar nem executar build.rs/proc-macros.
    let mut cmd = Command::new("cargo");
    cmd.args(["generate-lockfile", "--manifest-path"])
        .arg(root.join("Cargo.toml").as_str());
    if offline {
        cmd.arg("--offline");
    }
    match cmd.output() {
        Ok(o) if o.status.success() => Staging::Resolved { _guard: dir, root },
        Ok(o) => Staging::ResolveFailed {
            reason: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        },
        Err(e) => Staging::ResolveFailed {
            reason: format!("could not invoke `cargo generate-lockfile`: {e}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_manifest_is_not_applicable() {
        let toml = "[workspace]\nmembers=[\"a\"]\n";
        assert!(self_contained(toml).is_err());
    }

    #[test]
    fn inherited_fields_not_applicable() {
        let toml = "[package]\nname=\"x\"\nversion.workspace = true\n";
        assert!(self_contained(toml).is_err());
    }

    #[test]
    fn path_dep_not_applicable() {
        let toml = "[package]\nname=\"x\"\nversion=\"0.1.0\"\n[dependencies]\nfoo = { path = \"../foo\" }\n";
        assert!(self_contained(toml).is_err());
    }

    #[test]
    fn plain_manifest_is_self_contained() {
        let toml = "[package]\nname=\"x\"\nversion=\"0.1.0\"\n[dependencies]\nserde = \"1\"\n";
        assert!(self_contained(toml).is_ok());
    }
}
