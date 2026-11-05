//! Adaptador Seiva na CLI: liga uma extensão conhecida (ex.:
//! `seiva-console-admin`) à composição gerida, após a dependência ter sido
//! adicionada/validada. NUNCA corre hooks do crate — só interpreta o protocolo
//! declarativo do DX (dx-seiva) e edita ficheiros por âncoras.

use std::process::ExitCode;

use anyhow::{Context, Result};
use camino::Utf8Path;
use dx_manifest::{apply_transaction, FileWrite};
use dx_seiva::ExtensionSpec;

/// Lê `[package.metadata.dx.seiva] composition` do Cargo.toml.
fn composition_path(manifest_text: &str) -> Option<String> {
    let v: toml::Value = toml::from_str(manifest_text).ok()?;
    v.get("package")?
        .get("metadata")?
        .get("dx")?
        .get("seiva")?
        .get("composition")?
        .as_str()
        .map(str::to_string)
}

/// Integra uma extensão Seiva conhecida. `dep_verified` indica se a dependência
/// passou o gate (vs. aterrou como não-verificada). Devolve o código de saída.
pub fn integrate(
    root: &Utf8Path,
    manifest_path: &Utf8Path,
    spec: &ExtensionSpec,
    dep_verified: bool,
) -> Result<ExitCode> {
    eprintln!(
        "\n== Seiva adapter: {} (protocol v{}) ==",
        spec.crate_name, spec.protocol_version
    );
    eprintln!("  {}", spec.summary);
    if !dep_verified {
        eprintln!("  note: dependency landed NOT verified by the gate (run `dx audit`).");
    }

    let manifest_text = std::fs::read_to_string(manifest_path.as_std_path())
        .with_context(|| format!("read {manifest_path}"))?;
    let comp_rel = match composition_path(&manifest_text) {
        Some(p) => p,
        None => {
            // Composição NÃO gerida: não fingir integração (contrato §6).
            eprintln!(
                "  unmanaged composition (no [package.metadata.dx.seiva].composition).\n\
                 PARTIAL STATE: the dependency was added, but integration was NOT done.\n\
                 To integrate: use `dx new --template seiva` or add to a composition\n\
                 file the anchors `// dx:seiva:register:begin/end` and the line:\n\
                     {}",
                spec.register_line
            );
            return Ok(ExitCode::from(6));
        }
    };

    let comp_path = root.join(&comp_rel);
    let src = std::fs::read_to_string(comp_path.as_std_path())
        .with_context(|| format!("read composition {comp_path}"))?;
    let result = dx_seiva::integrate(&src, spec);

    if !result.missing_anchors.is_empty() {
        eprintln!(
            "  composition {comp_rel} missing anchors {:?}.\n\
             PARTIAL STATE: pending patch — add the markers and the line:\n\
                 {}",
            result.missing_anchors, spec.register_line
        );
        return Ok(ExitCode::from(6));
    }

    if !result.changed {
        eprintln!("  already integrated in {comp_rel} (idempotent — nothing to do).");
        return Ok(ExitCode::SUCCESS);
    }

    // Escrita transaccional (mesmo journal/lock do resto do DX).
    apply_transaction(
        root,
        &[FileWrite {
            path: comp_path.clone(),
            new_contents: result.new_source,
        }],
    )
    .context("write composition")?;
    eprintln!("  \u{2713} integrated in {comp_rel} (`register` anchor).");
    eprintln!("  files affected: {manifest_path} (dependency), {comp_rel} (composition).");
    eprintln!("  rebuild to wire it: `dx build`. No admin permissions; no passwords.");
    Ok(ExitCode::SUCCESS)
}
