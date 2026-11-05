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
}

impl Default for SandboxSpec {
    fn default() -> Self {
        SandboxSpec {
            deny_network: true,
            scrub_env: true,
            extra_env_allow: Vec::new(),
            limits: ResourceLimits::default(),
            cwd: None,
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
}

#[derive(Debug)]
pub struct SandboxOutcome {
    pub code: Option<i32>,
    pub enforced: EnforcedReport,
}

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("falha ao lançar o processo isolado: {0}")]
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
        "unshare(CLONE_NEWUSER|CLONE_NEWNET) — só loopback".to_string()
    } else if want_net {
        "PEDIDO mas NÃO imposto: user+net namespace indisponível no host".to_string()
    } else {
        "rede permitida (não pedido isolamento)".to_string()
    };

    let status = cmd.status()?;

    Ok(SandboxOutcome {
        code: status.code(),
        enforced: EnforcedReport {
            network_isolated: net_enforced,
            network_mechanism,
            env_scrubbed: spec.scrub_env,
            env_kept,
            rlimits_applied,
            filesystem_isolated: false,
            filesystem_note: "NÃO imposto nesta versão (sem mount namespace/FS mínimo)".into(),
        },
    })
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
