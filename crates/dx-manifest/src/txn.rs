//! Escrita transaccional com journal e recuperação de crash.
//!
//! Renomear vários ficheiros não é uma transacção atómica única (contrato §4),
//! por isso o DX escreve primeiro um *journal* com o estado final pretendido,
//! aplica cada escrita por `tmp + fsync + rename` (promoção atómica) e só então
//! apaga o journal. Se houver crash a meio, a próxima operação chama `recover`,
//! que completa as escritas em falta de forma idempotente (comparando hashes).
//!
//! Limite honesto: isto protege contra crash/interrupção e contra ler um
//! ficheiro parcial — **não** contra um atacante com controlo total do mesmo
//! utilizador/sistema (ver threat model).

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Write};

use crate::lock::WorkspaceLock;

/// Uma escrita de ficheiro a aplicar transaccionalmente.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileWrite {
    pub path: Utf8PathBuf,
    pub new_contents: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct JournalEntry {
    path: Utf8PathBuf,
    new_sha256: String,
    new_contents: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Journal {
    version: u32,
    writes: Vec<JournalEntry>,
}

fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    let digest = h.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn dx_dir(root: &Utf8Path) -> Utf8PathBuf {
    root.join(".dx")
}

fn journal_path(root: &Utf8Path) -> Utf8PathBuf {
    dx_dir(root).join("journal.json")
}

/// Escreve `contents` em `path` por tmp+fsync+rename (promoção atómica).
fn write_atomic(path: &Utf8Path, contents: &str) -> io::Result<()> {
    let parent = path.parent().unwrap_or(Utf8Path::new("."));
    // tmp no MESMO directório para garantir rename atómico (mesmo filesystem).
    let tmp = parent.join(format!(
        ".{}.dx-tmp",
        path.file_name().unwrap_or("manifest")
    ));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    if let Ok(dir) = fs::File::open(parent) {
        let _ = dir.sync_all(); // fsync do directório fixa a entrada do rename
    }
    Ok(())
}

/// Aplica um conjunto de escritas de forma transaccional sob o lock do workspace.
/// Idempotente perante crash: a próxima chamada a `recover` conclui o que faltar.
pub fn apply_transaction(root: &Utf8Path, writes: &[FileWrite]) -> io::Result<()> {
    if writes.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(dx_dir(root))?;
    let lock = WorkspaceLock::acquire(root)?;
    apply_transaction_locked(root, writes, &lock)
}

/// Como [`apply_transaction`], mas o chamador JÁ detém o lock e passa-o como
/// testemunha. É o que torna possível um **read-modify-write atómico** (B-1):
/// adquirir o lock, LER o estado em disco, modificá-lo e ESCREVER tudo sob o
/// mesmo lock — sem o re-adquirir (o flock não é reentrante no mesmo processo,
/// uma 2ª aquisição bloquearia). Um `dx add` concorrente deixa de perder uma
/// dependência: cada processo lê o estado actual sob o lock e re-aplica a sua
/// mudança.
pub fn apply_transaction_locked(
    root: &Utf8Path,
    writes: &[FileWrite],
    _lock: &WorkspaceLock,
) -> io::Result<()> {
    if writes.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(dx_dir(root))?;

    // Recuperar qualquer journal pendente antes de começar um novo.
    recover_locked(root)?;

    let journal = Journal {
        version: 1,
        writes: writes
            .iter()
            .map(|w| JournalEntry {
                path: w.path.clone(),
                new_sha256: sha256_hex(&w.new_contents),
                new_contents: w.new_contents.clone(),
            })
            .collect(),
    };
    let jpath = journal_path(root);
    let serialized = serde_json::to_string_pretty(&journal).map_err(io::Error::other)?;
    write_atomic(&jpath, &serialized)?;

    // Aplicar cada escrita. Um crash aqui deixa o journal para recover.
    for w in writes {
        write_atomic(&w.path, &w.new_contents)?;
    }

    // Commit: remover o journal.
    fs::remove_file(&jpath)?;
    if let Ok(dir) = fs::File::open(dx_dir(root)) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Recupera um journal pendente (completa escritas em falta). Seguro chamar
/// sempre no arranque de um comando mutador. Adquire o lock.
pub fn recover(root: &Utf8Path) -> io::Result<bool> {
    if !journal_path(root).exists() {
        return Ok(false);
    }
    fs::create_dir_all(dx_dir(root))?;
    let _lock = WorkspaceLock::acquire(root)?;
    recover_locked(root)
}

/// Como [`recover`], mas o chamador JÁ detém o lock (testemunha). Para um RMW que
/// quer recuperar um journal pendente ANTES de ler o estado em disco, sob o mesmo
/// lock que a escrita subsequente (B-1), sem re-adquirir.
pub fn recover_with_lock(root: &Utf8Path, _lock: &WorkspaceLock) -> io::Result<bool> {
    recover_locked(root)
}

/// Como `recover`, mas assume o lock já detido.
fn recover_locked(root: &Utf8Path) -> io::Result<bool> {
    let jpath = journal_path(root);
    if !jpath.exists() {
        return Ok(false);
    }
    let data = fs::read_to_string(&jpath)?;
    let journal: Journal = match serde_json::from_str(&data) {
        Ok(j) => j,
        Err(_) => {
            // Journal corrompido/parcial: nunca o tratamos como válido.
            // Removemo-lo; o manifesto original permanece intacto porque as
            // escritas de alvo são atómicas (tmp+rename).
            fs::remove_file(&jpath)?;
            return Ok(false);
        }
    };
    for entry in &journal.writes {
        let needs_write = match fs::read_to_string(&entry.path) {
            Ok(current) => sha256_hex(&current) != entry.new_sha256,
            Err(_) => true, // ficheiro ausente → escrever
        };
        if needs_write {
            write_atomic(&entry.path, &entry.new_contents)?;
        }
    }
    fs::remove_file(&jpath)?;
    Ok(true)
}
