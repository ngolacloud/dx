//! Testes de integração da CLI `dx`.
//!
//! Desde que o gate de segurança passou a correr em `dx add` (resolver grafo
//! candidato em staging → auditar → só então aterrar), a aterragem de uma
//! dependência de registry depende da rede/base. Para manter estes testes
//! deterministas:
//!
//! - A mecânica de manifesto (idempotência, git/rev, recuperação) usa a forma
//!   **NotApplicable** do staging (manifesto com dependência `path`), que NÃO
//!   toca na rede e aterra com código 6 (não-verificado).
//! - O comportamento do gate em si (pending/approved) está atrás de
//!   `DX_GATE_TESTS=1` porque precisa do cache do Cargo (serde) — corrido com
//!   evidência à parte. A lógica pura do veredicto está coberta por testes de
//!   unidade em `dx-security`.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;

fn dx() -> Command {
    Command::cargo_bin("dx").unwrap()
}

/// Projecto cujo manifesto tem uma dependência `path` — torna o staging
/// `NotApplicable`, pelo que `dx add` não toca na rede e aterra (código 6).
fn seed_notapplicable(dir: &Path) {
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nlocaldep = { path = \"./local\" }\n",
    )
    .unwrap();
}

/// Projecto auto-contido (dispara o gate real — usar só com `--offline` + env).
fn seed_plain(dir: &Path) {
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\n",
    )
    .unwrap();
}

#[test]
fn new_lib_scaffolds_files() {
    let tmp = tempfile::tempdir().unwrap();
    dx().current_dir(tmp.path())
        .args(["new", "minha-lib", "--lib"])
        .assert()
        .success();
    let root = tmp.path().join("minha-lib");
    assert!(root.join("Cargo.toml").exists());
    assert!(root.join("src/lib.rs").exists());
    assert!(root.join(".gitignore").exists());
    assert!(!root.join(".dx/journal.json").exists());
}

#[test]
fn new_refuses_nonempty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let existing = tmp.path().join("ocupado");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("algo.txt"), "x").unwrap();
    dx().current_dir(tmp.path())
        .args(["new", "ocupado", "--bin"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists and is not empty"));
}

#[test]
fn add_is_idempotent_byte_for_byte() {
    let tmp = tempfile::tempdir().unwrap();
    seed_notapplicable(tmp.path());

    // 1ª vez: staging NotApplicable → aterra sem rede (código 6 = não-verificado).
    dx().current_dir(tmp.path())
        .args(["add", "serde@1", "--features", "derive"])
        .assert()
        .code(6)
        .stderr(predicate::str::contains("pre-landing gate NOT applied"));
    let after_first = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
    assert!(after_first.contains("serde"));
    assert!(after_first.contains("derive"));

    // 2ª vez: no-op (retorna antes do gate), byte-a-byte igual.
    dx().current_dir(tmp.path())
        .args(["add", "serde@1", "--features", "derive"])
        .assert()
        .success()
        .stderr(predicate::str::contains("no changes"));
    let after_second = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
    assert_eq!(
        after_first, after_second,
        "add repetido não altera o manifesto"
    );

    // Ordem de features irrelevante → continua no-op.
    dx().current_dir(tmp.path())
        .args(["add", "serde@1", "--features", "derive,derive"])
        .assert()
        .success()
        .stderr(predicate::str::contains("no changes"));
    let after_third = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
    assert_eq!(after_first, after_third);
}

#[test]
fn plan_writes_nothing_and_shows_impact() {
    // Forma NotApplicable (dep path) → plano offline, sem rede; não escreve.
    let tmp = tempfile::tempdir().unwrap();
    seed_notapplicable(tmp.path());
    let before = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
    dx().current_dir(tmp.path())
        .args(["add", "serde@1", "--plan"])
        .assert()
        .code(6) // NotApplicable → grafo não analisável, mas não escreve
        .stderr(predicate::str::contains("dx add --plan"))
        .stderr(predicate::str::contains("Manifest change"));
    let after = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
    assert_eq!(before, after, "--plan não pode escrever");
}

#[test]
fn environment_capture_then_verify_matches() {
    let tmp = tempfile::tempdir().unwrap();
    seed_plain(tmp.path());
    dx().current_dir(tmp.path())
        .args(["environment", "capture"])
        .assert()
        .success();
    assert!(tmp.path().join("dx-environment.json").exists());
    dx().current_dir(tmp.path())
        .args(["environment", "verify"])
        .assert()
        .success()
        .stderr(predicate::str::contains("matches the recorded one"));
}

#[test]
fn environment_verify_without_file_errors() {
    let tmp = tempfile::tempdir().unwrap();
    seed_plain(tmp.path());
    dx().current_dir(tmp.path())
        .args(["environment", "verify"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "run `dx environment capture` first",
        ));
}

#[test]
fn update_without_lockfile_errors() {
    let tmp = tempfile::tempdir().unwrap();
    seed_plain(tmp.path());
    dx().current_dir(tmp.path())
        .args(["update", "--plan"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no Cargo.lock"));
}

#[test]
fn dry_run_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    seed_plain(tmp.path());
    let before = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
    // --dry-run retorna antes do gate; não toca na rede nem escreve.
    dx().current_dir(tmp.path())
        .args(["add", "anyhow@1", "--dry-run"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Plan"));
    let after = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
    assert_eq!(before, after, "--dry-run não pode escrever");
}

#[test]
fn source_conflict_refuses() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname=\"demo\"\nversion=\"0.1.0\"\n[dependencies]\nfoo = \"1\"\n",
    )
    .unwrap();
    dx().current_dir(tmp.path())
        .args([
            "add",
            "foo",
            "--git",
            "https://example/foo",
            "--rev",
            "deadbeef",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("different source"));
}

#[test]
fn typosquat_is_refused_with_suggestion() {
    let tmp = tempfile::tempdir().unwrap();
    seed_plain(tmp.path());
    dx().current_dir(tmp.path())
        .args(["add", "seiva-conosle-admin"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("seiva-console-admin"));
    let body = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
    assert!(!body.contains("conosle"));
}

#[test]
fn registry_without_version_is_rejected_with_guidance() {
    let tmp = tempfile::tempdir().unwrap();
    seed_plain(tmp.path());
    dx().current_dir(tmp.path())
        .args(["add", "anyhow"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("without a version"));
}

#[test]
fn remove_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname=\"demo\"\nversion=\"0.1.0\"\n[dependencies]\nserde = \"1\"\n",
    )
    .unwrap();
    dx().current_dir(tmp.path())
        .args(["remove", "serde"])
        .assert()
        .success();
    let body = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
    assert!(!body.contains("serde"));
    dx().current_dir(tmp.path())
        .args(["remove", "serde"])
        .assert()
        .success()
        .stderr(predicate::str::contains("no changes"));
}

#[test]
fn git_dep_with_rev_lands_in_manifest() {
    // NotApplicable (já há dep path) → não resolve git na rede; aterra o git dep.
    let tmp = tempfile::tempdir().unwrap();
    seed_notapplicable(tmp.path());
    dx().current_dir(tmp.path())
        .args([
            "add",
            "minha-lib",
            "--git",
            "https://github.com/org/minha-lib",
            "--rev",
            "abc123",
        ])
        .assert()
        .code(6);
    let body = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
    assert!(body.contains("git = \"https://github.com/org/minha-lib\""));
    assert!(body.contains("rev = \"abc123\""));
}

#[test]
fn not_applicable_lands_but_reports_unverified() {
    let tmp = tempfile::tempdir().unwrap();
    seed_notapplicable(tmp.path());
    dx().current_dir(tmp.path())
        .args(["add", "serde@1"])
        .assert()
        .code(6)
        .stderr(predicate::str::contains("Run `dx audit`"));
}

/// Journal pendente é recuperado antes da operação seguinte, sem perda.
#[test]
fn pending_journal_is_recovered_on_next_op() {
    let tmp = tempfile::tempdir().unwrap();
    seed_notapplicable(tmp.path());
    let dxdir = tmp.path().join(".dx");
    fs::create_dir_all(&dxdir).unwrap();
    let target = tmp.path().join("Cargo.toml");
    // Corpo recuperado mantém a dep path (continua NotApplicable → offline).
    let new_body = "[package]\nname=\"demo\"\nversion=\"0.1.0\"\n[dependencies]\nlocaldep = { path = \"./local\" }\nrecovered = \"1\"\n";
    let sha = sha256_hex(new_body);
    let journal = format!(
        "{{\"version\":1,\"writes\":[{{\"path\":{:?},\"new_sha256\":{:?},\"new_contents\":{:?}}}]}}",
        target.to_string_lossy(),
        sha,
        new_body
    );
    fs::write(dxdir.join("journal.json"), journal).unwrap();

    dx().current_dir(tmp.path())
        .args(["add", "serde@1"])
        .assert()
        .code(6);

    let body = fs::read_to_string(&target).unwrap();
    assert!(
        body.contains("recovered = \"1\""),
        "journal aplicado: {body}"
    );
    assert!(body.contains("serde"), "add prosseguiu após recover");
    assert!(!dxdir.join("journal.json").exists(), "journal consumido");
}

// --- Testes do gate real (precisam do cache do Cargo p/ serde) -------------
// Activar com DX_GATE_TESTS=1. Correm offline (serde em cache) + --offline.

fn gate_tests_enabled() -> bool {
    std::env::var("DX_GATE_TESTS")
        .map(|v| v == "1")
        .unwrap_or(false)
}

#[test]
fn gate_pending_when_db_expired_blocks_landing() {
    if !gate_tests_enabled() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    seed_plain(tmp.path());
    // DX_MAX_DB_AGE_SECS=0 ⇒ qualquer base é "expirada" ⇒ PENDENTE (exit 5).
    dx().current_dir(tmp.path())
        .env("DX_MAX_DB_AGE_SECS", "0")
        .args(["add", "serde@1", "--offline"])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("did NOT land"));
    let body = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
    assert!(!body.contains("serde"), "pending não pode aterrar");
}

#[test]
fn gate_approved_when_db_fresh_lands() {
    if !gate_tests_enabled() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    seed_plain(tmp.path());
    // Janela enorme ⇒ base "válida" ⇒ serde (limpo) ⇒ APROVADO (exit 0).
    dx().current_dir(tmp.path())
        .env("DX_MAX_DB_AGE_SECS", "99999999999")
        .args(["add", "serde@1", "--offline"])
        .assert()
        .success()
        .stderr(predicate::str::contains("APPROVED"));
    let body = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
    assert!(body.contains("serde"));
}

fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
