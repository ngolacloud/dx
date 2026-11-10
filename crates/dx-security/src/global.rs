//! Global, tamper-evident decision cache (ADR-0024).
//!
//! The per-root cache (`verdict::save`/`load`, `<root>/.dx/security/`) is reused
//! only within one project. This module adds a **global** store at
//! `~/.dx/verdicts/`, keyed by the SAME `CacheKey` digest, so a verdict computed
//! once is reused by any checkout with the **identical** `Cargo.lock` — other
//! worktrees, CI runners, teammates — without re-running `cargo-audit`.
//!
//! Because the key includes the full-lockfile sha, reuse requires the *same*
//! lockfile; the benefit is CI/teams/monorepos and worktrees, not arbitrary
//! unrelated projects (honest scope — see ADR-0024).
//!
//! **Tamper-evidence.** A shared cache is an attack surface: a forged `approved`
//! verdict must not be trusted. Each stored verdict carries an HMAC-SHA256 over
//! `"<key.digest>|<decision>"`, keyed by a machine-local secret at
//! `~/.dx/verdict.key` (32 bytes from `/dev/urandom`, mode 0600, created once).
//! On load, the MAC is recomputed and checked in constant time; any mismatch
//! makes the entry a **cache miss** (the gate re-evaluates — fail-closed). The
//! key file is the trust anchor: share it within a trusted team to share
//! verdicts; otherwise it is machine-local anti-tamper.

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::verdict::{CacheKey, Verdict};

const SHA256_BLOCK: usize = 64;

/// HMAC-SHA256 (RFC 2104).
fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    // Shorten keys longer than the block size, then zero-pad to the block.
    let mut k = if key.len() > SHA256_BLOCK {
        Sha256::digest(key).to_vec()
    } else {
        key.to_vec()
    };
    k.resize(SHA256_BLOCK, 0);

    let mut ipad = [0x36u8; SHA256_BLOCK];
    let mut opad = [0x5cu8; SHA256_BLOCK];
    for i in 0..SHA256_BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(msg);
    let inner = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner);
    outer.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Constant-time byte comparison (no early return on first mismatch).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

fn home() -> Utf8PathBuf {
    Utf8PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

/// Directory of the global verdict store (`DX_VERDICT_DIR` or `~/.dx/verdicts`).
pub fn global_dir() -> Utf8PathBuf {
    match std::env::var("DX_VERDICT_DIR") {
        Ok(d) if !d.is_empty() => Utf8PathBuf::from(d),
        _ => home().join(".dx/verdicts"),
    }
}

fn key_path() -> Utf8PathBuf {
    match std::env::var("DX_VERDICT_KEY") {
        Ok(p) if !p.is_empty() => Utf8PathBuf::from(p),
        _ => home().join(".dx/verdict.key"),
    }
}

/// Load the machine-local HMAC key, creating it (32 bytes, mode 0600) if absent.
fn load_or_create_key() -> std::io::Result<Vec<u8>> {
    let path = key_path();
    if let Ok(bytes) = std::fs::read(path.as_std_path()) {
        if bytes.len() >= 16 {
            return Ok(bytes);
        }
    }
    use std::io::Read;
    let mut buf = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent.as_std_path())?;
    }
    std::fs::write(path.as_std_path(), buf)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path.as_std_path(), std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(buf.to_vec())
}

/// The trust-critical message the MAC authenticates: this key → this decision.
fn mac_message(verdict: &Verdict) -> String {
    format!("{}|{}", verdict.key.digest(), verdict.decision.as_str())
}

#[derive(Serialize, Deserialize)]
struct Signed {
    mac: String,
    verdict: Verdict,
}

fn store_path(key: &CacheKey) -> Utf8PathBuf {
    global_dir().join(format!("decision-{}.json", key.digest()))
}

/// Store a verdict in the global cache, signed with the machine-local key.
pub fn save_global(verdict: &Verdict) -> std::io::Result<()> {
    let key = load_or_create_key()?;
    let mac = hex(&hmac_sha256(&key, mac_message(verdict).as_bytes()));
    let signed = Signed {
        mac,
        verdict: verdict.clone(),
    };
    let dir = global_dir();
    std::fs::create_dir_all(dir.as_std_path())?;
    let data = serde_json::to_string_pretty(&signed).map_err(std::io::Error::other)?;
    std::fs::write(store_path(&verdict.key).as_std_path(), data)
}

/// Load a verdict from the global cache **only** if the exact key matches AND
/// the HMAC verifies. Any mismatch (wrong key, tampered content, wrong signing
/// key) is treated as a miss — the gate re-evaluates (fail-closed).
pub fn load_global(key: &CacheKey) -> Option<Verdict> {
    let hkey = load_or_create_key().ok()?;
    let data = std::fs::read_to_string(store_path(key).as_std_path()).ok()?;
    let signed: Signed = serde_json::from_str(&data).ok()?;
    if &signed.verdict.key != key {
        return None;
    }
    let expect = hex(&hmac_sha256(&hkey, mac_message(&signed.verdict).as_bytes()));
    if ct_eq(expect.as_bytes(), signed.mac.as_bytes()) {
        Some(signed.verdict)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbStatus;
    use crate::policy::Policy;
    use crate::verdict::{evaluate, CacheKey};

    fn valid_db() -> DbStatus {
        DbStatus {
            path: "/x".into(),
            last_commit_unix: Some(1000),
            age_secs: Some(10),
            max_age_secs: 86400,
            valid: true,
            detail: "ok".into(),
        }
    }

    fn key(lock: &str) -> CacheKey {
        CacheKey {
            lockfile_sha: lock.into(),
            policy_fp: "p".into(),
            db_snapshot: "s".into(),
            scanner_version: "v".into(),
        }
    }

    // These tests mutate process-global env vars, so they must not run
    // concurrently with one another.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // Isolate HOME-derived paths to a temp dir for each test.
    fn with_env(dir: &camino::Utf8Path, f: impl FnOnce()) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("DX_VERDICT_DIR", dir.join("verdicts").as_str());
        std::env::set_var("DX_VERDICT_KEY", dir.join("verdict.key").as_str());
        f();
        std::env::remove_var("DX_VERDICT_DIR");
        std::env::remove_var("DX_VERDICT_KEY");
    }

    #[test]
    fn hmac_matches_rfc_2104_vector() {
        // RFC 4231 test case 1: key=0x0b*20, data="Hi There".
        let mac = hmac_sha256(&[0x0b; 20], b"Hi There");
        assert_eq!(
            hex(&mac),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn global_roundtrip_hits_on_identical_key() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = camino::Utf8Path::from_path(tmp.path()).unwrap();
        with_env(dir, || {
            let v = evaluate(vec![], &Policy::default(), valid_db(), key("lockA"));
            save_global(&v).unwrap();
            assert!(load_global(&key("lockA")).is_some());
            // Different lockfile ⇒ different key ⇒ miss.
            assert!(load_global(&key("lockB")).is_none());
        });
    }

    #[test]
    fn flipped_decision_is_rejected() {
        use crate::audit::Finding;
        use crate::policy::Category;
        let tmp = tempfile::tempdir().unwrap();
        let dir = camino::Utf8Path::from_path(tmp.path()).unwrap();
        with_env(dir, || {
            // A BLOCKED verdict (vulnerability), signed for "blocked".
            let finding = Finding {
                category: Category::Vulnerability,
                package: "x".into(),
                version: "1".into(),
                advisory_id: Some("RUSTSEC-0000-0000".into()),
                title: "t".into(),
                url: None,
                dependency_path: None,
            };
            let v = evaluate(vec![finding], &Policy::default(), valid_db(), key("lockA"));
            assert_eq!(v.decision, crate::verdict::Decision::Blocked);
            save_global(&v).unwrap();
            // Forge an approval: flip the decision in the file without re-signing.
            let path = store_path(&key("lockA"));
            let text = std::fs::read_to_string(path.as_std_path()).unwrap();
            let forged = text.replace("\"blocked\"", "\"approved\"");
            assert_ne!(forged, text, "decision should have been present to flip");
            std::fs::write(path.as_std_path(), forged).unwrap();
            // MAC was computed over blocked → forged approved must be rejected.
            assert!(
                load_global(&key("lockA")).is_none(),
                "forged decision must be a cache miss"
            );
        });
    }

    #[test]
    fn wrong_signing_key_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = camino::Utf8Path::from_path(tmp.path()).unwrap();
        with_env(dir, || {
            let v = evaluate(vec![], &Policy::default(), valid_db(), key("lockA"));
            save_global(&v).unwrap();
            // Replace the signing key: previously-signed entry no longer verifies.
            std::fs::write(dir.join("verdict.key").as_std_path(), [9u8; 32]).unwrap();
            assert!(load_global(&key("lockA")).is_none());
        });
    }
}
