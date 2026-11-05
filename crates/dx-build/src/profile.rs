//! Perfis de build LLVM, aplicados por `cargo --config` (sem tocar no
//! Cargo.toml do utilizador).
//!
//! Princípio do contrato §7: **nunca impor `lto="fat"`/`codegen-units=1` como
//! optimização universal**. O default equilibrado é ThinLTO + codegen-units=16;
//! `throughput` (fat-LTO + cg1) é **opt-in** e documentado como lento.
//!
//! Também do §7: não mexemos em `panic`/`overflow-checks`/definições de
//! segurança só para ganhar números; os perfis só tocam opt-level/LTO/
//! codegen-units/strip.

use serde::Serialize;

/// Um perfil de build nomeado.
#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    pub name: &'static str,
    pub description: &'static str,
    /// Perfil Cargo base: "dev" ou "release".
    pub cargo_profile: &'static str,
    /// Overrides `KEY=VALUE` a passar via `cargo --config` (valores TOML).
    pub config_overrides: Vec<String>,
    /// Nota/limitação a mostrar ao utilizador.
    pub note: &'static str,
}

impl Profile {
    /// Constrói os argumentos extra para o `cargo build` deste perfil.
    pub fn cargo_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if self.cargo_profile == "release" {
            args.push("--release".to_string());
        }
        for ov in &self.config_overrides {
            args.push("--config".to_string());
            args.push(ov.clone());
        }
        args
    }

    /// Caminho relativo do directório de artefactos (`target/<dir>`).
    pub fn target_subdir(&self) -> &'static str {
        if self.cargo_profile == "release" {
            "release"
        } else {
            "debug"
        }
    }
}

/// Perfil embutido pelo nome.
pub fn builtin(name: &str) -> Option<Profile> {
    Some(match name {
        "dev" => Profile {
            name: "dev",
            description: "Fast compile, no optimization (Cargo default).",
            cargo_profile: "dev",
            config_overrides: vec![],
            note: "incremental; for iteration, not for measuring performance.",
        },
        "release" => Profile {
            name: "release",
            description: "Balanced release: opt-level 3, ThinLTO, cg=16.",
            cargo_profile: "release",
            config_overrides: vec![
                "profile.release.opt-level=3".into(),
                "profile.release.lto=\"thin\"".into(),
                "profile.release.codegen-units=16".into(),
            ],
            note: "recommended default; keeps symbols (no strip).",
        },
        "throughput" => Profile {
            name: "throughput",
            description: "Maximum throughput: opt-level 3, fat-LTO, cg=1.",
            cargo_profile: "release",
            config_overrides: vec![
                "profile.release.opt-level=3".into(),
                "profile.release.lto=\"fat\"".into(),
                "profile.release.codegen-units=1".into(),
            ],
            note: "OPT-IN: fat-LTO+cg=1 is slow to compile and is NOT the default.",
        },
        "size" => Profile {
            name: "size",
            description: "Minimum size: opt-level z, ThinLTO, cg=1, strip.",
            cargo_profile: "release",
            config_overrides: vec![
                "profile.release.opt-level=\"z\"".into(),
                "profile.release.lto=\"thin\"".into(),
                "profile.release.codegen-units=1".into(),
                "profile.release.strip=\"symbols\"".into(),
            ],
            note: "strip removes symbols: worse diagnostics in exchange for size.",
        },
        _ => return None,
    })
}

/// Nomes de todos os perfis embutidos.
pub fn all_names() -> &'static [&'static str] {
    &["dev", "release", "throughput", "size"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_is_thin_lto_not_fat() {
        let p = builtin("release").unwrap();
        let joined = p.config_overrides.join(" ");
        assert!(joined.contains("lto=\"thin\""));
        assert!(
            !joined.contains("fat"),
            "release NÃO pode ser fat-LTO por default"
        );
        assert!(joined.contains("codegen-units=16"));
    }

    #[test]
    fn throughput_is_opt_in_fat() {
        let p = builtin("throughput").unwrap();
        let joined = p.config_overrides.join(" ");
        assert!(joined.contains("lto=\"fat\""));
        assert!(joined.contains("codegen-units=1"));
        assert!(p.note.to_lowercase().contains("opt-in"));
    }

    #[test]
    fn no_profile_touches_panic_or_overflow() {
        for name in all_names() {
            let p = builtin(name).unwrap();
            let joined = p.config_overrides.join(" ");
            assert!(!joined.contains("panic"), "{name} não deve mexer em panic");
            assert!(
                !joined.contains("overflow"),
                "{name} não deve mexer em overflow-checks"
            );
        }
    }

    #[test]
    fn cargo_args_release_has_flag_and_configs() {
        let p = builtin("release").unwrap();
        let args = p.cargo_args();
        assert_eq!(args[0], "--release");
        assert!(args.iter().any(|a| a == "--config"));
    }

    #[test]
    fn dev_has_no_release_flag() {
        let p = builtin("dev").unwrap();
        assert!(!p.cargo_args().contains(&"--release".to_string()));
    }
}
