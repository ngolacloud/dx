# ADR-0017 — `dx environment`: reproducible build-environment record

- Estado: Aceite (implementado; crate `dx-env`)
- Data: 2026-11-05
- Contexto: directriz de produto do dono (P1 — ambiente reproduzível além do
  Cargo.lock; cenários on-premises N'GolaCloud)

## Decisão

Um `Cargo.lock` fixa o grafo de dependências, mas **não** todo o ambiente de
build. `dx environment` regista e verifica esse ambiente:

- `dx environment capture` → escreve `dx-environment.json` (na raiz, para ser
  **commitado** e partilhado em CI/equipas) com: versão do `rustc`, target do
  host, versão do `cargo`, `RUSTFLAGS`/`CARGO_BUILD_TARGET`, ferramentas
  auxiliares (`cargo-audit`, `cargo-deny`, `cc`), **evidência de segurança**
  (fingerprint da política + snapshot/validade da advisory-db) e o **sha256 do
  Cargo.lock** (liga o ambiente ao grafo).
- `dx environment verify` → recomputa o ambiente do host e reporta **drift**.
  Drift crítico (rustc/target/cargo/flags/ferramentas/lockfile/política) falha
  (exit 4); o snapshot da advisory-db é **informacional** (muda com o tempo por
  desenho).

## Honestidade

- **Não** é reprodutibilidade byte-a-byte — é um **registo e verificação de
  drift**. Byte-a-byte só se anuncia depois de demonstrada.
- Cobre o que é medível no host; dependências nativas arbitrárias e imagem de
  build não são enumeradas exaustivamente (trabalho futuro: `dx bundle export`).

## Prova

- Unit (`dx-env`, 5 testes): sem diff quando igual; rustc/lockfile → drift
  crítico; ferramenta ausente → crítico; snapshot da advisory-db → não-crítico;
  JSON roundtrip.
- Integração (CLI): capture→verify bate; verify sem ficheiro falha com instrução.
- Ao vivo: capture regista toolchain/tools/advisory-db/lockfile; verify bate
  (exit 0); alterar o Cargo.lock → DRIFT crítico (exit 4).
