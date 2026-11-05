# DX

Experiência de ferramentas Rust inspirada no [UV](https://docs.astral.sh/uv/):
clara, rápida e consistente, **respeitando os mecanismos nativos de Rust**.
Cargo e rustc continuam a resolver e compilar; o DX acrescenta idempotência,
um gate de segurança da cadeia de dependências, relatórios e perfis de build.

- **Executável:** `dx`
- **Crate de publicação:** `dx-cli` (o nome `dx` está ocupado em crates.io — ver
  [ADR-0003](docs/adr/0003-nome-publicado-vs-executavel.md))
- **Opcional:** não exige Seiva, DORM, Kubernetes nem Delonix. O Seiva é um
  consumidor opcional; o DX é opcional para qualquer projecto.

## Estado (2026-11-05)

| Área | Estado |
|------|--------|
| Contrato Cargo + mutações idempotentes (`new`/`init`/`add`/`remove`) | **Implementado + testado** |
| `--dry-run`, plano, journal, lock, recuperação de crash | **Implementado + testado** |
| Detecção de typosquatting (sinal) | **Implementado + testado** |
| Passthrough estruturado p/ Cargo (`build`/`check`/`run`/`test`/`fmt`/`lint`) | **Implementado** |
| Gate de segurança (`dx audit`/`dx doctor`, cargo-audit, validade da base, veredicto, cache) | **Implementado + testado** |
| Gate ligado ao `dx add` (staging isolado sem build.rs; só aterra se aprovado) | **Implementado + testado** |
| `cargo-deny` (licenças/origens/bans) + política de origem anti dependency-confusion | **Implementado + testado** |
| Integridade (`dx fetch`: checksum vs lockfile) + extracção segura (traversal/symlink/zip-bomb) | **Implementado + testado** |
| `fetch`/`update`/`release prepare` | **Planeado** — stub honesto (exit 3) |
| Perfis LLVM / PGO / `release prepare` | **Planeado (inc.4)** |
| Adaptador Seiva / `seiva-console-admin` | **Planeado (inc.5)** |

Ver o relatório de progresso em [`docs/progress.md`](docs/progress.md).

## Exemplo

```console
$ dx new minha-lib --lib
$ cd minha-lib
$ dx add serde@1 --features derive      # idempotente: repetir é no-op
$ dx add serde@1 --features derive --dry-run   # mostra o plano, não escreve
$ dx build
```

O `Cargo.toml` gerado é Cargo puro — compila e testa **sem** o Dx (critério de
aceitação "Cargo sem DX"). Invocar o Cargo directamente contorna o gate DX, por
desenho.

## Desenho

- [ADR-0001 — Arquitectura](docs/adr/0001-arquitectura.md)
- [ADR-0002 — Contrato Cargo](docs/adr/0002-contrato-cargo.md)
- [ADR-0003 — Nome publicado vs executável](docs/adr/0003-nome-publicado-vs-executavel.md)
- [ADR-0004 — Idempotência, journal, recuperação](docs/adr/0004-idempotencia-journal.md)
- [ADR-0005 — Gate de segurança](docs/adr/0005-gate-seguranca.md)
- [Threat model](docs/threat-model.md)
- [CLI](docs/cli.md)

## Desenvolvimento

```console
$ cargo build
$ cargo test          # 13 unit + 10 integração
```
