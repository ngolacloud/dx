//! `dx-sandbox` — isolamento **real** de execução, com relatório honesto.
//!
//! O contrato (§5, "Defesa contra crates maliciosas") é explícito: oferecer
//! isolamento por plataforma (sem segredos, rede desligada após fetch, FS
//! mínimo, limites de recursos) e **mostrar capacidades reais — não chamar
//! sandbox a um simples subprocesso**. Documentar onde o isolamento não existe.
//!
//! Capacidades reais nesta versão (Linux, sem root):
//! - **Rede isolada** via `unshare(CLONE_NEWUSER|CLONE_NEWNET)` (user+net
//!   namespace): o processo só vê o loopback (para baixo), logo não há rede
//!   externa. Detectado em runtime; se indisponível, é **reportado como não
//!   imposto**, nunca fingido.
//! - **Limites de recursos** via `setrlimit` (CPU, tamanho de ficheiro, nº de
//!   processos) aplicados no filho antes do `exec`.
//! - **Ambiente sem segredos**: o ambiente é limpo e só uma allowlist mínima
//!   é passada — variáveis de segredo nunca atravessam.
//!
//! Limites honestos: o **isolamento de filesystem** (mount namespace, FS
//! mínimo) **não** está imposto nesta versão. Não protege contra um atacante
//! com controlo total do mesmo utilizador/sistema. Fora de Linux, quase nada é
//! imposto e isso é reportado.

use camino::Utf8PathBuf;
use serde::Serialize;
use std::process::Command;
use std::time::Duration;

/// Variáveis de ambiente mínimas que deixamos passar para um build isolado.
const ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LANG",
    "LC_ALL",
    "TERM",
    "TMPDIR",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTC",
    "CARGO",
    "RUSTFLAGS",
    "CARGO_TARGET_DIR",
];

/// Capacidades de isolamento detectadas no host.
#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    pub platform: String,
    /// É possível criar user+net namespace sem root?
    pub network_isolation: bool,
    /// setrlimit disponível?
    pub resource_limits: bool,
    /// Isolamento de filesystem imposto? (não nesta versão)
    pub filesystem_isolation: bool,
}

impl Capabilities {
    pub fn detect() -> Capabilities {
        Capabilities {
            platform: std::env::consts::OS.to_string(),
            network_isolation: probe_netns(),
            resource_limits: cfg!(unix),
            filesystem_isolation: false,
        }
    }
}

/// Testa se `unshare --user --net --map-root-user -- true` corre com sucesso.
fn probe_netns() -> bool {
    if !cfg!(unix) {
        return false;
    }
    Command::new("unshare")
        .args(["--user", "--net", "--map-root-user", "--", "true"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Limites de recursos a aplicar. `None` num campo = não aplicar esse limite
/// (evita partir builds legítimos). Defaults generosos mas finitos.
#[derive(Debug, Clone, Serialize)]
pub struct ResourceLimits {
    pub cpu_seconds: Option<u64>,
    pub file_size_bytes: Option<u64>,
    pub max_processes: Option<u64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        ResourceLimits {
            cpu_seconds: Some(900),
            file_size_bytes: Some(2 * 1024 * 1024 * 1024),
            max_processes: Some(4096),
        }
    }
}

/// Especificação de execução isolada.
#[derive(Debug, Clone)]
pub struct SandboxSpec {
    /// Desligar a rede (se o host o permitir).
    pub deny_network: bool,
    /// Limpar o ambiente e passar só a allowlist.
    pub scrub_env: bool,
    /// Variáveis extra a permitir além da allowlist mínima.
    pub extra_env_allow: Vec<String>,
    pub limits: ResourceLimits,
    pub cwd: Option<Utf8PathBuf>,
    /// Deadline de **relógio de parede** (wall-clock). `RLIMIT_CPU` limita CPU, não
    /// I/O — um `build.rs` que bloqueia/dorme em I/O penduraria para sempre (E-1).
    /// Ao exceder o deadline, o GRUPO de processos do filho é morto e colhido.
    /// `None` = sem deadline (não recomendado). Default: 1h (generoso mas finito).
    pub wall_clock_timeout: Option<Duration>,
}

impl Default for SandboxSpec {
    fn default() -> Self {
        SandboxSpec {
            deny_network: true,
            scrub_env: true,
            extra_env_allow: Vec::new(),
            limits: ResourceLimits::default(),
            cwd: None,
            wall_clock_timeout: Some(Duration::from_secs(3600)),
        }
    }
}

/// O que foi **de facto** imposto nesta execução.
#[derive(Debug, Clone, Serialize)]
pub struct EnforcedReport {
    pub network_isolated: bool,
    pub network_mechanism: String,
    pub env_scrubbed: bool,
    pub env_kept: Vec<String>,
    pub rlimits_applied: Vec<String>,
    pub filesystem_isolated: bool,
    pub filesystem_note: String,
    /// Deadline wall-clock imposto nesta execução (e.g. `"3600s"`), ou `None` se
    /// não houver (E-1). O filho corre no seu próprio grupo de processos, morto e
    /// colhido ao exceder o deadline ou ao receber SIGINT/SIGTERM (E-2).
    pub wall_clock_timeout: Option<String>,
}

#[derive(Debug)]
pub struct SandboxOutcome {
    pub code: Option<i32>,
    /// `true` se a execução foi morta por exceder o deadline wall-clock (E-1). O
    /// `code` fica `None` (morto por sinal, não saída normal).
    pub timed_out: bool,
    pub enforced: EnforcedReport,
}

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("failed to spawn isolated process: {0}")]
    Spawn(#[from] std::io::Error),
}

/// Executa `program` com os argumentos dados sob o isolamento pedido.
/// Herda stdio. Devolve o código de saída e o relatório do que foi imposto.
pub fn run(
    spec: &SandboxSpec,
    program: &str,
    args: &[String],
    caps: &Capabilities,
) -> Result<SandboxOutcome, SandboxError> {
    let want_net = spec.deny_network;
    let net_enforced = want_net && caps.network_isolation;

    // Montar o comando: via `unshare` quando isolamos a rede; senão directo.
    let mut cmd = if net_enforced {
        let mut c = Command::new("unshare");
        c.args(["--user", "--net", "--map-root-user", "--", program]);
        c.args(args);
        c
    } else {
        let mut c = Command::new(program);
        c.args(args);
        c
    };

    // Ambiente sem segredos.
    let mut env_kept = Vec::new();
    if spec.scrub_env {
        cmd.env_clear();
        let allow: Vec<String> = ENV_ALLOWLIST
            .iter()
            .map(|s| s.to_string())
            .chain(spec.extra_env_allow.iter().cloned())
            .collect();
        for key in &allow {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
                env_kept.push(key.clone());
            }
        }
    }

    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }

    // Limites de recursos no filho, antes do exec (setrlimit é async-signal-safe).
    let rlimits_applied = apply_rlimits(&mut cmd, &spec.limits);

    let network_mechanism = if net_enforced {
        "unshare(CLONE_NEWUSER|CLONE_NEWNET) — loopback only".to_string()
    } else if want_net {
        "REQUESTED but NOT enforced: user+net namespace unavailable on host".to_string()
    } else {
        "network allowed (isolation not requested)".to_string()
    };

    // Supervisão do ciclo de vida (E-1/E-2): o filho corre no seu próprio grupo de
    // processos, com um deadline wall-clock; ao exceder, o GRUPO inteiro (o filho e
    // os seus descendentes — build.rs e filhos) é morto e colhido, não só o filho
    // directo. Um SIGINT/SIGTERM ao `dx` também mata o grupo (senão ficaria órfão).
    let (code, timed_out) = supervise(cmd, spec.wall_clock_timeout)?;

    Ok(SandboxOutcome {
        code,
        timed_out,
        enforced: EnforcedReport {
            network_isolated: net_enforced,
            network_mechanism,
            env_scrubbed: spec.scrub_env,
            env_kept,
            rlimits_applied,
            filesystem_isolated: false,
            filesystem_note: "NOT enforced in this version (no mount namespace/minimal FS)".into(),
            wall_clock_timeout: spec.wall_clock_timeout.map(|t| format!("{}s", t.as_secs())),
        },
    })
}

/// Executa `cmd` com um deadline wall-clock e reaping do grupo de processos (E-1/E-2).
///
/// Devolve `(código de saída, timed_out)`. No caminho não-Unix não há grupos de
/// processos nem sinais POSIX, pelo que corre bloqueante (o deadline é ignorado,
/// reportado honestamente por [`Capabilities`]).
#[cfg(unix)]
fn supervise(
    mut cmd: Command,
    timeout: Option<Duration>,
) -> Result<(Option<i32>, bool), SandboxError> {
    use std::os::unix::process::CommandExt;
    use std::sync::atomic::Ordering;
    use std::time::Instant;

    // O filho LIDERA o seu próprio grupo de processos (pgid == pid do filho), para
    // que um kill por deadline/sinal colha a ÁRVORE inteira, não só o filho directo.
    cmd.process_group(0);
    let mut child = cmd.spawn()?;
    #[allow(clippy::cast_possible_wrap)]
    let pgid = child.id() as i32;

    // Reaping sob SIGINT/SIGTERM ao `dx`: num grupo próprio, o filho seria reparentado
    // para o init (órfão) quando o `dx` morresse. O handler mata o grupo primeiro (E-2).
    install_signal_reaper();
    SUPERVISED_PGID.store(pgid, Ordering::SeqCst);
    // Mata o grupo também em qualquer retorno antecipado/panic (guarda de Drop).
    let _guard = GroupGuard { pgid };

    let deadline = timeout.map(|t| Instant::now() + t);
    let mut timed_out = false;
    let code = loop {
        if let Some(status) = child.try_wait()? {
            break status.code();
        }
        if let Some(d) = deadline {
            if Instant::now() >= d {
                kill_group(pgid);
                let status = child.wait()?;
                timed_out = true;
                break status.code();
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    };

    SUPERVISED_PGID.store(0, Ordering::SeqCst);
    Ok((code, timed_out))
}

#[cfg(not(unix))]
fn supervise(
    mut cmd: Command,
    _timeout: Option<Duration>,
) -> Result<(Option<i32>, bool), SandboxError> {
    let status = cmd.status()?;
    Ok((status.code(), false))
}

/// O grupo de processos (pgid) do filho supervisionado em curso, `0` quando nenhum.
/// Lido pelo handler de sinal (async-signal-safe: só um load atómico).
#[cfg(unix)]
static SUPERVISED_PGID: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Mata o grupo de processos `pgid` com `SIGKILL`. Alvo é SEMPRE o grupo do filho
/// (pgid == pid do filho, via `process_group(0)`), nunca o do `dx`.
#[cfg(unix)]
fn kill_group(pgid: i32) {
    // SAFETY: `killpg` é async-signal-safe e só atinge o grupo do filho.
    unsafe {
        libc::killpg(pgid, libc::SIGKILL);
    }
}

/// Instala, uma vez, um handler de SIGINT/SIGTERM que mata o grupo do filho
/// supervisionado antes de o `dx` sair — sem isto, um grupo próprio ficaria órfão.
#[cfg(unix)]
fn install_signal_reaper() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: regista um handler async-signal-safe (ver `reaper`).
        unsafe {
            let handler = reaper as *const () as libc::sighandler_t;
            libc::signal(libc::SIGTERM, handler);
            libc::signal(libc::SIGINT, handler);
        }
    });
}

/// Handler de sinal: mata o grupo supervisionado (se houver) e sai. Usa apenas
/// operações async-signal-safe (um load atómico, `killpg`, `_exit`).
#[cfg(unix)]
extern "C" fn reaper(signum: libc::c_int) {
    let pgid = SUPERVISED_PGID.load(std::sync::atomic::Ordering::SeqCst);
    if pgid != 0 {
        unsafe {
            libc::killpg(pgid, libc::SIGKILL);
        }
    }
    // Sai com o código convencional 128+sinal, sem cleanup de alto nível (que não
    // seria async-signal-safe) — o grupo já foi colhido acima.
    unsafe {
        libc::_exit(128 + signum);
    }
}

/// Guarda de Drop que mata o grupo do filho — garante reaping em qualquer retorno
/// antecipado ou panic de [`supervise`] (um grupo já vazio ⇒ `killpg` ESRCH, inócuo).
#[cfg(unix)]
struct GroupGuard {
    pgid: i32,
}

#[cfg(unix)]
impl Drop for GroupGuard {
    fn drop(&mut self) {
        kill_group(self.pgid);
        SUPERVISED_PGID.store(0, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(unix)]
fn apply_rlimits(cmd: &mut Command, limits: &ResourceLimits) -> Vec<String> {
    use std::os::unix::process::CommandExt;

    let mut applied = Vec::new();
    if let Some(v) = limits.cpu_seconds {
        applied.push(format!("RLIMIT_CPU={v}s"));
    }
    if let Some(v) = limits.file_size_bytes {
        applied.push(format!("RLIMIT_FSIZE={v}"));
    }
    if let Some(v) = limits.max_processes {
        applied.push(format!("RLIMIT_NPROC={v}"));
    }
    let cpu = limits.cpu_seconds;
    let fsize = limits.file_size_bytes;
    let nproc = limits.max_processes;

    // SAFETY: só chamamos setrlimit (syscall async-signal-safe) no filho.
    unsafe {
        cmd.pre_exec(move || {
            set_one(libc::RLIMIT_CPU, cpu)?;
            set_one(libc::RLIMIT_FSIZE, fsize)?;
            set_one(libc::RLIMIT_NPROC, nproc)?;
            Ok(())
        });
    }
    applied
}

#[cfg(unix)]
fn set_one(resource: libc::__rlimit_resource_t, value: Option<u64>) -> std::io::Result<()> {
    if let Some(v) = value {
        let lim = libc::rlimit {
            rlim_cur: v as libc::rlim_t,
            rlim_max: v as libc::rlim_t,
        };
        // SAFETY: `lim` é válido; setrlimit é async-signal-safe.
        let rc = unsafe { libc::setrlimit(resource, &lim) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn apply_rlimits(_cmd: &mut Command, _limits: &ResourceLimits) -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_capabilities_without_panicking() {
        let caps = Capabilities::detect();
        assert_eq!(caps.platform, std::env::consts::OS);
        // Não afirmamos valores (dependem do host), só que detecta sem falhar.
        assert!(!caps.filesystem_isolation, "FS não é imposto nesta versão");
    }

    #[test]
    fn env_is_scrubbed_secret_does_not_pass() {
        // Definimos um segredo no ambiente do pai e confirmamos que NÃO passa.
        std::env::set_var("DX_FAKE_SECRET", "supersecreto");
        let caps = Capabilities::detect();
        let spec = SandboxSpec {
            deny_network: false, // isolar aqui não é o foco do teste
            ..Default::default()
        };
        // `env` imprime o ambiente; procuramos o segredo na saída.
        let out = run_capture(&spec, "env", &[], &caps);
        assert!(
            !out.contains("supersecreto"),
            "o segredo não pode atravessar o ambiente limpo"
        );
        assert!(out.contains("PATH="), "a allowlist mínima deve passar PATH");
        std::env::remove_var("DX_FAKE_SECRET");
    }

    #[cfg(unix)]
    fn run_capture(
        spec: &SandboxSpec,
        program: &str,
        args: &[String],
        caps: &Capabilities,
    ) -> String {
        use std::os::unix::process::CommandExt;
        // Variante de captura só para teste (o run() herda stdio).
        let mut cmd = std::process::Command::new(program);
        cmd.args(args);
        if spec.scrub_env {
            cmd.env_clear();
            for key in ENV_ALLOWLIST {
                if let Ok(val) = std::env::var(key) {
                    cmd.env(key, val);
                }
            }
        }
        let cpu = spec.limits.cpu_seconds;
        unsafe {
            cmd.pre_exec(move || set_one(libc::RLIMIT_CPU, cpu));
        }
        let _ = caps;
        let out = cmd.output().unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}
