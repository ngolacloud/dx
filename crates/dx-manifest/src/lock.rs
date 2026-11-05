//! Lock de workspace para serializar operações concorrentes do DX.
//!
//! Duas invocações de `dx add` em paralelo não podem corromper o manifesto
//! (contrato §4). Usamos um lock de ficheiro exclusivo e consultivo (fs4) em
//! `.dx/dx.lock`. É consultivo: protege contra dois processos DX, não contra
//! quem edite Cargo.toml por fora ignorando o lock.

use camino::Utf8Path;
use fs4::fs_std::FileExt;
use std::fs::{self, File};
use std::io;

/// Guarda RAII do lock exclusivo do workspace. Liberta ao sair de scope.
pub struct WorkspaceLock {
    _file: File,
}

impl WorkspaceLock {
    /// Adquire o lock exclusivo, bloqueando até estar disponível.
    pub fn acquire(root: &Utf8Path) -> io::Result<WorkspaceLock> {
        let dir = root.join(".dx");
        fs::create_dir_all(&dir)?;
        let path = dir.join("dx.lock");
        let file = File::create(&path)?;
        FileExt::lock_exclusive(&file)?;
        Ok(WorkspaceLock { _file: file })
    }

    /// Tenta adquirir sem bloquear; `None` se já estiver detido por outro.
    pub fn try_acquire(root: &Utf8Path) -> io::Result<Option<WorkspaceLock>> {
        let dir = root.join(".dx");
        fs::create_dir_all(&dir)?;
        let path = dir.join("dx.lock");
        let file = File::create(&path)?;
        match FileExt::try_lock_exclusive(&file) {
            Ok(true) => Ok(Some(WorkspaceLock { _file: file })),
            Ok(false) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl Drop for WorkspaceLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self._file);
    }
}
