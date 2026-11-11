//! `dx update` — explainable, security-driven, recoverable dependency updates.
//!
//! `--plan` shows what `cargo update` would change (computed in staging, real
//! lockfile untouched). `--security` runs the gate before and after the update
//! and reports which advisories the update CLEARS and which remain without a
//! compatible fix (we explain the block, we do NOT swap the library for a
//! different one). `--apply` applies the updated lockfile transactionally,
//! validates with the gate (and `--test`), and RESTORES the previous lockfile
//! if validation fails (recovery).

use std::collections::BTreeMap;
use std::process::{Command, ExitCode};

use anyhow::{bail, Context, Result};
use camino::Utf8PathBuf;
use dx_integrity::all_packages;
use dx_manifest::{apply_transaction, recover, FileWrite};
use dx_security::{run_gate, Decision, Policy, Verdict};

use crate::{security, stage};

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

/// Sidecar DURÁVEL com o Cargo.lock anterior enquanto um `update --apply` está a
/// validar (C-2). A sua existência = "apply pendente/interrompido".
const ROLLBACK_SIDECAR: &str = ".dx/update-rollback.lock";

/// Antes de comitar o lockfile novo, guarda o actual num sidecar durável — para que
/// um crash durante a validação (o `--test` demora minutos) não deixe um lockfile
/// NÃO-validado em efeito sem nada para recuperar, violando a ADR-0016 (C-2).
fn stash_rollback(root: &camino::Utf8Path, current_lock: &str) -> Result<()> {
    apply_transaction(
        root,
        &[FileWrite {
            path: root.join(ROLLBACK_SIDECAR),
            new_contents: current_lock.to_string(),
        }],
    )
    .context("stash the rollback lockfile")
}

/// Restaura o Cargo.lock a partir do sidecar e limpa-o (rollback durável). Usado
/// quando a validação falha E na recuperação de um apply interrompido.
fn restore_rollback(root: &camino::Utf8Path) -> Result<()> {
    let sidecar = root.join(ROLLBACK_SIDECAR);
    let backup =
        std::fs::read_to_string(sidecar.as_std_path()).context("read the rollback sidecar")?;
    apply_transaction(
        root,
        &[FileWrite {
            path: root.join("Cargo.lock"),
            new_contents: backup,
        }],
    )
    .context("restore the previous Cargo.lock")?;
    let _ = std::fs::remove_file(sidecar.as_std_path());
    Ok(())
}

/// Apply validado com sucesso → o lockfile novo fica; descarta o sidecar.
fn clear_rollback(root: &camino::Utf8Path) {
    let _ = std::fs::remove_file(root.join(ROLLBACK_SIDECAR).as_std_path());
}

/// Se um `update --apply` anterior foi interrompido (sidecar presente), faz
/// rollback do lockfile não-validado ANTES de prosseguir. Devolve `true` se
/// recuperou. É o que torna o backup durável útil: o estado não-validado nunca
/// fica permanentemente em efeito.
fn recover_pending_rollback(root: &camino::Utf8Path) -> Result<bool> {
    if root.join(ROLLBACK_SIDECAR).exists() {
        restore_rollback(root)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn locate() -> Result<(Utf8PathBuf, Utf8PathBuf)> {
    let cwd = std::env::current_dir().context("CWD")?;
    let mut dir = Utf8PathBuf::from_path_buf(cwd).map_err(|_| anyhow::anyhow!("non-UTF8 CWD"))?;
    loop {
        let m = dir.join("Cargo.toml");
        if m.exists() {
            return Ok((dir.clone(), m));
        }
        if !dir.pop() {
            bail!("no Cargo.toml found from the current directory");
        }
    }
}

/// name -> sorted set of versions present in a lockfile.
fn version_map(pkgs: &[dx_integrity::PackageEntry]) -> BTreeMap<String, Vec<String>> {
    let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in pkgs {
        m.entry(p.name.clone()).or_default().push(p.version.clone());
    }
    for v in m.values_mut() {
        v.sort();
        v.dedup();
    }
    m
}

/// Version changes between two lockfiles (name, old, new).
fn changes(
    before: &[dx_integrity::PackageEntry],
    after: &[dx_integrity::PackageEntry],
) -> Vec<(String, String, String)> {
    let b = version_map(before);
    let a = version_map(after);
    let mut out = Vec::new();
    for (name, av) in &a {
        let bv = b.get(name);
        if bv.map(|x| x != av).unwrap_or(true) {
            let old = bv.map(|x| x.join(",")).unwrap_or_else(|| "—".into());
            let new = av.join(",");
            if Some(&new) != bv.map(|x| x.join(",")).as_ref() {
                out.push((name.clone(), old, new));
            }
        }
    }
    out.sort();
    out
}

/// Blocking findings of a verdict as a set of "pkg@ver — title".
fn blocking_set(v: &Verdict) -> Vec<String> {
    v.blocking
        .iter()
        .filter_map(|&i| v.findings.get(i))
        .map(|f| {
            format!(
                "{}@{} ({}) — {}",
                f.package,
                f.version,
                f.category.as_str(),
                f.title
            )
        })
        .collect()
}

pub fn cmd_update(
    name: Option<&str>,
    plan: bool,
    security: bool,
    apply: bool,
    offline: bool,
    test: bool,
) -> Result<ExitCode> {
    // Default action when no flag is given: --plan (explain, do not change).
    let (do_plan, do_security, do_apply) = if !plan && !security && !apply {
        (true, false, false)
    } else {
        (plan, security, apply)
    };

    let (root, manifest_path) = locate()?;
    recover(&root).ok();
    let manifest_toml = std::fs::read_to_string(manifest_path.as_std_path())
        .with_context(|| format!("read {manifest_path}"))?;
    let before = all_packages(&root.join("Cargo.lock")).unwrap_or_default();
    if before.is_empty() {
        bail!("no Cargo.lock to update (run `dx add`/resolve first)");
    }

    let scope = name.map(|n| format!(" -p {n}")).unwrap_or_default();
    eprintln!("== dx update{scope} ==");

    // Keep the whole Staging alive (its TempDir guard) until the end of this
    // function — otherwise the staging directory is deleted before we read it.
    let staging_result = stage::resolve_update(&root, &manifest_toml, name, offline);
    let staging = match &staging_result {
        stage::Staging::Resolved { root, .. } => root.clone(),
        stage::Staging::NotApplicable { reason } => {
            bail!("cannot stage update for this project shape ({reason}); use a self-contained manifest");
        }
        stage::Staging::ResolveFailed { reason } => {
            bail!("could not compute the update:\n  {reason}");
        }
    };
    let staging_lock = staging.join("Cargo.lock");
    let after = all_packages(&staging_lock).unwrap_or_default();
    let chg = changes(&before, &after);

    if chg.is_empty() {
        eprintln!("Already up to date — no compatible version changes.");
    } else {
        eprintln!("Version changes ({}):", chg.len());
        for (n, old, new) in &chg {
            eprintln!("  {n}: {old} -> {new}");
        }
    }

    let policy = Policy::from_env();

    if do_security || do_apply {
        // Gate before (current) and after (updated).
        let before_v = run_gate(&root, &root, &policy, security::now_unix())
            .context("gate on current graph")?
            .verdict;
        let after_v = run_gate(&root, &staging, &policy, security::now_unix())
            .context("gate on updated graph")?
            .verdict;
        let before_block = blocking_set(&before_v);
        let after_block = blocking_set(&after_v);
        let cleared: Vec<&String> = before_block
            .iter()
            .filter(|f| !after_block.contains(f))
            .collect();
        let remaining = &after_block;

        eprintln!("\nSecurity effect of the update:");
        if cleared.is_empty() {
            eprintln!("  cleared: none");
        } else {
            eprintln!("  cleared ({}):", cleared.len());
            for c in &cleared {
                eprintln!("    - {c}");
            }
        }
        if remaining.is_empty() {
            eprintln!("  remaining blockers: none");
        } else {
            eprintln!(
                "  remaining blockers ({}) — no compatible fix via update:",
                remaining.len()
            );
            for r in remaining {
                eprintln!("    ! {r}");
            }
            eprintln!(
                "  (DX explains the block and does NOT swap the library for a different one; \
                 consider a patch/alternative with its own review.)"
            );
        }
    }

    if !do_apply {
        if do_plan && !do_security {
            eprintln!(
                "\n(plan: nothing written. Use --security to check advisories, --apply to apply.)"
            );
        }
        return Ok(ExitCode::SUCCESS);
    }

    // --- Apply with validation and recovery ---
    if chg.is_empty() {
        eprintln!("\nNothing to apply.");
        return Ok(ExitCode::SUCCESS);
    }
    let real_lock = root.join("Cargo.lock");
    // C-2: recuperar um `update --apply` anterior INTERROMPIDO (crash durante a
    // validação) antes de mutar — faz rollback do lockfile não-validado que ficou
    // em efeito, usando o sidecar durável. Sem isto, o estado não-validado persistia.
    if recover_pending_rollback(&root).context("recover an interrupted update")? {
        eprintln!(
            "note: a previous `update --apply` was interrupted during validation — the unvalidated Cargo.lock was rolled back."
        );
    }
    let backup = std::fs::read_to_string(real_lock.as_std_path()).unwrap_or_default();
    let updated =
        std::fs::read_to_string(staging_lock.as_std_path()).context("read staged lockfile")?;

    eprintln!("\nApplying updated lockfile...");
    // C-2: guardar o lockfile actual num sidecar DURÁVEL ANTES de comitar o novo, de
    // modo que um crash na validação (minutos) deixe sempre algo para recuperar
    // (ADR-0016) — o `backup` deixa de viver só em memória.
    stash_rollback(&root, &backup).context("stash rollback before applying")?;
    apply_transaction(
        &root,
        &[FileWrite {
            path: real_lock.clone(),
            new_contents: updated,
        }],
    )
    .context("write updated Cargo.lock")?;

    // Validate on the real project.
    let v = run_gate(&root, &root, &policy, security::now_unix())
        .context("validation gate")?
        .verdict;
    let gate_ok = v.decision == Decision::Approved;
    let tests_ok = if test {
        eprintln!("Running `cargo test`...");
        Command::new(cargo())
            .arg("test")
            .current_dir(root.as_std_path())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    } else {
        true
    };

    if gate_ok && tests_ok {
        // Validado: o lockfile novo fica; descarta-se o sidecar de rollback (C-2).
        clear_rollback(&root);
        eprintln!(
            "\n\u{2713} Applied and validated (gate approved{}).",
            if test { ", tests passed" } else { "" }
        );
        Ok(ExitCode::SUCCESS)
    } else {
        // Recuperação: restaura o lockfile anterior a partir do sidecar DURÁVEL
        // (não de uma cópia só em memória) e limpa o sidecar (C-2).
        restore_rollback(&root).context("restore previous Cargo.lock")?;
        eprintln!(
            "\n\u{2717} Validation failed ({}{}). Previous Cargo.lock RESTORED — project unchanged.",
            if gate_ok { "gate ok" } else { "gate not approved" },
            if test && !tests_ok { ", tests failed" } else { "" }
        );
        Ok(ExitCode::from(4))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dx_integrity::PackageEntry;

    // --- C-2: rollback durável do `update --apply` ---------------------------

    #[test]
    fn an_interrupted_apply_rolls_back_the_unvalidated_lockfile() {
        let tmp = tempfile::tempdir().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        std::fs::write(root.join("Cargo.lock").as_std_path(), "OLD").unwrap();

        // `apply` guarda o lockfile actual durável e comita o novo…
        stash_rollback(root, "OLD").unwrap();
        std::fs::write(root.join("Cargo.lock").as_std_path(), "NEW-unvalidated").unwrap();
        // …e aqui o processo é MORTO durante a validação: o sidecar fica.
        assert!(
            root.join(ROLLBACK_SIDECAR).exists(),
            "sidecar durável presente"
        );

        // O apply seguinte recupera: faz rollback do lockfile não-validado.
        assert!(recover_pending_rollback(root).unwrap());
        assert_eq!(
            std::fs::read_to_string(root.join("Cargo.lock").as_std_path()).unwrap(),
            "OLD",
            "o Cargo.lock não-validado foi revertido para o anterior"
        );
        assert!(
            !root.join(ROLLBACK_SIDECAR).exists(),
            "sidecar limpo após recuperar"
        );
        // Sem sidecar, uma 2ª recuperação é no-op.
        assert!(!recover_pending_rollback(root).unwrap());
    }

    #[test]
    fn a_successful_apply_clears_the_rollback_sidecar() {
        let tmp = tempfile::tempdir().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        std::fs::write(root.join("Cargo.lock").as_std_path(), "OLD").unwrap();
        stash_rollback(root, "OLD").unwrap();
        std::fs::write(root.join("Cargo.lock").as_std_path(), "NEW-validated").unwrap();

        clear_rollback(root);
        assert!(
            !root.join(ROLLBACK_SIDECAR).exists(),
            "sidecar descartado no sucesso"
        );
        // O lockfile validado permanece.
        assert_eq!(
            std::fs::read_to_string(root.join("Cargo.lock").as_std_path()).unwrap(),
            "NEW-validated"
        );
    }

    #[test]
    fn a_failed_validation_restores_from_the_durable_sidecar() {
        let tmp = tempfile::tempdir().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        std::fs::write(root.join("Cargo.lock").as_std_path(), "OLD").unwrap();
        stash_rollback(root, "OLD").unwrap();
        std::fs::write(root.join("Cargo.lock").as_std_path(), "NEW-bad").unwrap();

        // Validação falhou → restaura do sidecar durável (não de memória).
        restore_rollback(root).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("Cargo.lock").as_std_path()).unwrap(),
            "OLD"
        );
        assert!(!root.join(ROLLBACK_SIDECAR).exists());
    }

    fn mk(n: &str, v: &str) -> PackageEntry {
        PackageEntry {
            name: n.into(),
            version: v.into(),
            source: None,
            checksum: None,
        }
    }

    #[test]
    fn changes_detects_version_bump() {
        let before = vec![mk("smallvec", "1.6.0"), mk("serde", "1.0.0")];
        let after = vec![mk("smallvec", "1.13.2"), mk("serde", "1.0.0")];
        let c = changes(&before, &after);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0], ("smallvec".into(), "1.6.0".into(), "1.13.2".into()));
    }

    #[test]
    fn changes_empty_when_identical() {
        let p = vec![mk("serde", "1.0.0")];
        assert!(changes(&p, &p).is_empty());
    }
}
