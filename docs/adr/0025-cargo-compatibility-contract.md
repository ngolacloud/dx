# ADR-0025 — Contrato de compatibilidade com o Cargo (maturidade)

- Estado: Aceite (implementado em `feat/trust-layer-overhead-zero`)
- Data: 2026-11-10
- Contexto: plano de melhorias, Fase 5 — maturidade/confiança

## Problema

O DX **compõe** o Cargo (ADR-0002/0020): não reimplementa o resolver nem o
compilador, **delega** num conjunto de interfaces `cargo`/`rustc`. O reverso é
**acoplamento** — uma mudança no JSON do `cargo metadata` ou na forma do
`cargo tree` pode partir o DX num ponto qualquer a jusante, sem aviso claro.
Faltava fixar esse contrato e testá-lo.

## Decisão

### 1. Contrato explícito (crate `dx-compat`)

As interfaces de que o DX depende, fixadas em código (sincronizadas com os call
sites reais):

| Interface | Para quê | Obrigatória |
|---|---|---|
| `cargo metadata --format-version 1` | grafo/packages (plan, ordem topológica de release, adaptador Seiva) | sim |
| `cargo tree -i / -d / -e` | `why`/`explain` e o caminho inverso nos achados do audit | sim |
| `cargo generate-lockfile` | resolver o grafo candidato em staging sem compilar | sim |
| `cargo update` | grafo mínimo actualizado (`dx update`) | sim |
| `cargo vendor` | bundle offline verificável (`dx bundle`) | não |
| `rustc -vV` | proveniência de toolchain (`dx toolchain`, `dx environment`) | sim |

O DX fixa o **intervalo testado**: MSRV `1.82` (do workspace) a `MAX_TESTED`
`1.96`. Uma versão mais nova é **permitida mas sinalizada** («newer-than-tested»),
nunca silenciosamente confiada; abaixo do MSRV é recusada.

### 2. `dx compat` — sonda o host contra o contrato

Classifica a versão do Cargo e sonda cada interface (formato do
`cargo metadata --format-version 1` num projecto real; disponibilidade dos
subcomandos via `--help`; `rustc -vV`). Sai `4` se uma interface **obrigatória**
estiver partida ou o Cargo for demasiado antigo. Uma linha-resumo aparece
também no `dx doctor`.

### 3. Matriz de CI (`.github/workflows/compat.yml`)

Corre a suite + `dx compat` em `1.82`/`stable`/`beta`/`nightly` (beta/nightly
podem falhar — aviso precoce de drift do Cargo ainda não lançado), semanalmente
e em cada PR. Assim, drift de formato do Cargo é apanhado **aqui**.

## Âmbito e itens adiados (honestidade)

Esta fase cobre o eixo de maior risco de confiança — o acoplamento ao Cargo. Os
outros dois itens de «maturidade» ficam explicitamente adiados, cada um com o seu
ADR futuro:

- **Reprodutibilidade byte-a-byte do `dx bundle`** em CI (reconstruir e comparar
  hashes) — o `dx bundle verify` já detecta adulteração; falta a prova de
  rebuild idêntico.
- **Nome de descoberta** (`dx` vs `ngoladx-cli`, ADR-0003) — reavaliar um atalho
  de instalação mais limpo quando/se os nomes no crates.io libertarem.

## Anti-objectivos

- Não reimplementar nenhuma destas interfaces — apenas verificá-las.
- Não bloquear em versões mais novas do que as testadas (sinalizar, não recusar).
- Não deixar o contrato divergir dos call sites reais (manter a tabela e o crate
  em sincronia).
