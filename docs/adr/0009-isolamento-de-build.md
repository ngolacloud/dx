# ADR-0009 — Isolamento de build (capacidades reais)

- Estado: Aceite (implementado; crate `dx-sandbox`)
- Data: 2026-11-05
- Contexto do contrato: §5 "Defesa contra crates maliciosas" — "oferece
  isolamento por plataforma... Mostra capacidades reais; não chamar sandbox a
  um simples subprocesso. Documenta limitações onde o isolamento não existe."

## Decisão

Crate `dx-sandbox` executa comandos (tipicamente `cargo build`) com isolamento
**real**, aplicando só primitivas de kernel que existem no host e **reportando
exactamente o que foi imposto** — nunca chamando sandbox a um subprocesso nu.

### Capacidades (Linux, sem root)

- **Rede isolada**: `unshare(CLONE_NEWUSER|CLONE_NEWNET)` (user+net namespace).
  O processo só vê o loopback (para baixo) ⇒ sem rede externa. Detectado em
  runtime (`unshare --user --net --map-root-user -- true`); se indisponível, é
  **reportado como não imposto**.
- **Limites de recursos**: `setrlimit` no filho antes do `exec` (async-signal-
  safe): `RLIMIT_CPU`, `RLIMIT_FSIZE`, `RLIMIT_NPROC`. Defaults finitos mas
  generosos para não partir builds legítimos.
- **Ambiente sem segredos**: `env_clear()` + allowlist mínima (PATH, HOME, …).
  Variáveis de segredo nunca atravessam.

### Build hermético

`dx build --isolated` corre `cargo build --offline` sob o sandbox e desliga
qualquer `rustc-wrapper` global dependente de rede (ex.: sccache) via
`--config build.rustc-wrapper=""` — senão falharia a contactar a rede.

## Prova

- Unit/integração (`dx-sandbox`, 4 testes): detecção de capacidades; segredo
  não atravessa o ambiente; **rede mesmo cortada** (getent externo falha dentro
  do namespace); rlimits/env reportados como impostos.
- Ao vivo: `dx doctor` → `isolamento (linux): rede=true rlimits=true
  filesystem=false`. `dx build --isolated` → binário produzido com rede isolada;
  antes de desligar o wrapper, o sccache falhou com *Network unreachable* —
  evidência directa de que a rede estava cortada.

## Limites honestos

- **Isolamento de filesystem NÃO imposto** nesta versão (sem mount namespace/FS
  mínimo) — reportado como `false`.
- Não protege contra um atacante com controlo total do mesmo utilizador/sistema.
- Fora de Linux, quase nada é imposto; as capacidades reportam isso.
- O isolamento aplica-se a `dx build --isolated`; invocar `cargo` directamente
  contorna-o (DX é opcional, por desenho).
