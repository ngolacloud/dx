//! Testes de integração da CLI `dx` — cobrem os cenários de aceitação do
//! incremento 1 (contrato §9, linhas aplicáveis): idempotência, dry-run,
//! conflito de origem, typosquatting, scaffolding e recuperação de journal.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;

fn dx() -> Command {
    Command::cargo_bin("dx").unwrap()
}

fn seed_project(dir: &Path) {
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
    // Sem journal pendente após sucesso.
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
        .stderr(predicate::str::contains("já existe e não está vazio"));
}

#[test]
fn add_is_idempotent_byte_for_byte() {
    let tmp = tempfile::tempdir().unwrap();
    seed_project(tmp.path());

    dx().current_dir(tmp.path())
        .args(["add", "serde@1", "--features", "derive"])
        .assert()
        .success();
    let after_first = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
    assert!(after_first.contains("serde"));
    assert!(after_first.contains("derive"));

    // Repetir com a mesma intenção → no-op, byte-a-byte igual.
    dx().current_dir(tmp.path())
        .args(["add", "serde@1", "--features", "derive"])
        .assert()
        .success()
        .stderr(predicate::str::contains("sem alterações"));
    let after_second = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
    assert_eq!(
        after_first, after_second,
        "add repetido não pode alterar o manifesto"
    );

    // Ordem de features não altera o estado.
    dx().current_dir(tmp.path())
        .args(["add", "serde@1", "--features", "derive,derive"])
        .assert()
        .success()
        .stderr(predicate::str::contains("sem alterações"));
    let after_third = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
    assert_eq!(after_first, after_third);
}

#[test]
fn dry_run_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    seed_project(tmp.path());
    let before = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
    dx().current_dir(tmp.path())
        .args(["add", "anyhow@1", "--dry-run"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Plano"));
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
        .stderr(predicate::str::contains("origem diferente"));
}

#[test]
fn typosquat_is_refused_with_suggestion() {
    let tmp = tempfile::tempdir().unwrap();
    seed_project(tmp.path());
    dx().current_dir(tmp.path())
        .args(["add", "seiva-conosle-admin"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("seiva-console-admin"));
    // E não escreveu nada.
    let body = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
    assert!(!body.contains("conosle"));
}

#[test]
fn registry_without_version_is_rejected_with_guidance() {
    let tmp = tempfile::tempdir().unwrap();
    seed_project(tmp.path());
    dx().current_dir(tmp.path())
        .args(["add", "anyhow"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("sem versão"));
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
    // Remover de novo → no-op.
    dx().current_dir(tmp.path())
        .args(["remove", "serde"])
        .assert()
        .success()
        .stderr(predicate::str::contains("sem alterações"));
}

#[test]
fn git_dep_with_rev_lands_in_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    seed_project(tmp.path());
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
        .success();
    let body = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
    assert!(body.contains("git = \"https://github.com/org/minha-lib\""));
    assert!(body.contains("rev = \"abc123\""));
}

/// Simula um crash: deixa um journal pendente e confirma que a operação
/// seguinte o recupera (completa a escrita) sem perder dados.
#[test]
fn pending_journal_is_recovered_on_next_op() {
    let tmp = tempfile::tempdir().unwrap();
    seed_project(tmp.path());
    // Forjar um journal pendente que escreve um Cargo.toml com `recovered = "1"`.
    let dxdir = tmp.path().join(".dx");
    fs::create_dir_all(&dxdir).unwrap();
    let target = tmp.path().join("Cargo.toml");
    let new_body =
        "[package]\nname=\"demo\"\nversion=\"0.1.0\"\n[dependencies]\nrecovered = \"1\"\n";
    let sha = sha256_hex(new_body);
    let journal = format!(
        "{{\"version\":1,\"writes\":[{{\"path\":{:?},\"new_sha256\":{:?},\"new_contents\":{:?}}}]}}",
        target.to_string_lossy(),
        sha,
        new_body
    );
    fs::write(dxdir.join("journal.json"), journal).unwrap();

    // Uma operação idempotente qualquer dispara o recover primeiro.
    dx().current_dir(tmp.path())
        .args(["add", "serde@1"])
        .assert()
        .success();

    let body = fs::read_to_string(&target).unwrap();
    assert!(
        body.contains("recovered = \"1\""),
        "journal devia ter sido aplicado: {body}"
    );
    // serde também entrou (a operação prosseguiu após o recover).
    assert!(body.contains("serde"));
    // Journal consumido.
    assert!(!dxdir.join("journal.json").exists());
}

// sha256 em hex — espelho mínimo do helper interno do dx-manifest para o teste.
fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
