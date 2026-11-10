# ADR-0022 — `dx toolchain`: pin de toolchain por projecto (análogo `uv python`)

- Estado: Aceite (implementado em `feat/trust-layer-overhead-zero`)
- Data: 2026-11-10
- Contexto: plano de melhorias, Fase 2 — fechar o maior buraco face ao UV

## Problema

O UV fechou a stack ao gerir o interpretador (`uv python install/pin`). No DX,
o maior buraco equivalente era a **toolchain**: o `dx environment capture/verify`
já *regista* `rustc`/`cargo` e detecta drift, mas não havia forma de **fixar**
nem **instalar** a toolchain por projecto a partir do DX — a remediação ficava
a cargo do utilizador, a correr `rustup` à mão.

## Decisão

Adicionar `dx toolchain list|install|pin|use`, **compondo** o `rustup`
(ADR-0002/0020), nunca o substituindo:

- **O pin é o ficheiro nativo `rust-toolchain.toml`.** Logo `cargo`/`rustup`
  honram-no **sem** o DX (critério «Cargo sem DX»). O DX só escreve/actualiza o
  `[toolchain] channel`, preservando chaves não-geridas (`profile`, etc.) e
  `components`/`targets` já fixados (escrita format-preserving via `toml_edit`).
- **Proveniência em `dx-toolchain.lock`.** O DX regista o que o canal
  **resolveu** (ex.: `1.96` → `rustc 1.96.1`, commit-hash, commit-date, host),
  lendo `rustc -vV`. Permite detectar que o *mesmo* pin passou a resolver para
  um rustc *diferente* (canal móvel como `stable`).
- **`dx environment verify` aponta aqui.** Perante drift crítico de
  `rustc`/`cargo`/`host_target`, sugere `dx toolchain use <canal-do-pin>` (ou
  `dx toolchain list` se não houver pin).
- **Sem fallback.** Se o `rustup` estiver ausente, o DX recusa honestamente
  («install rustup»), em vez de adivinhar ou reimplementar a gestão de toolchains.

### Regra dura: um `pin` nunca despoleta um download

O `rustup` auto-instala a toolchain fixada na invocação seguinte de
`cargo`/`rustc`. Para o DX, **registar proveniência não pode disparar um
download**: o `dx toolchain pin` só sonda a proveniência se o canal estiver
**exactamente** instalado (`rustup` trata `1.96` e `1.96.0` como toolchains
distintas — o DX casa só `canal` ou `canal-<host-triple>`, nunca `1.96.0` para
`1.96`). Se não estiver instalado, escreve o pin e imprime uma nota a mandar
correr `dx toolchain install`/`use` — sem descarregar nada.

- `install <v>`: `rustup toolchain install <v>` + proveniência exacta.
- `pin <v>`: escreve o pin; proveniência só se já instalado (senão nota).
- `use <v>`: instala (se faltar) + fixa + proveniência (resolve atalhos via
  `rustc -vV` no directório, p.ex. `1.96` → `1.96.1`).
- `list`: toolchains instaladas + pin do projecto + proveniência registada.

## Consequências

- O DX passa a cobrir o eixo «ambiente de toolchain» que faltava face ao UV,
  sem reimplementar o `rustup`.
- O pin é portátil e funciona em CI/equipas mesmo sem o DX.
- A proveniência dá ao `environment verify` uma remediação accionável.

## Anti-objectivos

- Não reimplementar o `rustup` (download/gestão de componentes).
- Não introduzir um formato de pin próprio — usa-se o `rust-toolchain.toml`.
- Nunca disparar um download a partir de `pin`/sondagem de proveniência.
