//! `dx-build` — perfis de build LLVM, medição e PGO.
//!
//! Usa o backend LLVM da toolchain Rust suportada (não construímos LLVM).
//! Perfis aplicados por `cargo --config` sem tocar no Cargo.toml do utilizador.
//! Medição real (tempo/memória/tamanho). PGO com `llvm-profdata` compatível.

pub mod measure;
pub mod pgo;
pub mod profile;

pub use measure::{file_size, run_measured, Measurement};
pub use profile::{all_names, builtin, Profile};

/// Contrato de CPU-alvo. Releases distribuídas **não** usam `target-cpu=native`
/// por omissão (contrato §7): um binário assim só corre na microarquitectura do
/// host que o compilou.
pub fn target_cpu_rustflag(native: bool) -> Option<String> {
    if native {
        Some("-Ctarget-cpu=native".to_string())
    } else {
        None
    }
}
