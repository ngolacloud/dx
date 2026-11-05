//! Prova comportamental do isolamento de rede.
//!
//! Quando o host suporta user+net namespaces, um comando que precisa de rede
//! (resolução DNS externa) **tem** de falhar dentro do sandbox. É determinista
//! independentemente de o host ter ou não rede: dentro do namespace só há
//! loopback (para baixo).

use dx_sandbox::{run, Capabilities, SandboxSpec};

#[test]
fn network_is_really_cut_inside_sandbox() {
    let caps = Capabilities::detect();
    if !caps.network_isolation {
        eprintln!("host sem user+net namespace — teste não aplicável (reportado, não fingido)");
        return;
    }
    let spec = SandboxSpec {
        deny_network: true,
        ..Default::default()
    };
    // getent hosts de um nome externo precisa de DNS → sem rede, falha.
    let out = run(
        &spec,
        "getent",
        &["hosts".to_string(), "github.com".to_string()],
        &caps,
    )
    .expect("sandbox deve lançar");
    assert!(out.enforced.network_isolated, "rede devia estar isolada");
    assert_ne!(
        out.code,
        Some(0),
        "resolução externa tem de falhar dentro do namespace de rede"
    );
}

#[test]
fn rlimits_and_env_reported_as_enforced() {
    let caps = Capabilities::detect();
    let spec = SandboxSpec {
        deny_network: false,
        ..Default::default()
    };
    let out = run(&spec, "true", &[], &caps).expect("lança");
    assert!(out.enforced.env_scrubbed);
    assert!(!out.enforced.rlimits_applied.is_empty());
    assert!(
        !out.enforced.filesystem_isolated,
        "FS honestamente não imposto"
    );
}
