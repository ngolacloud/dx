# ADR-0026 — Pré-screen por crate (fast-BLOCK), sem enfraquecer o gate

- Estado: Aceite (implementado em `feat/trust-layer-overhead-zero`)
- Data: 2026-11-10
- Contexto: item adiado do ADR-0024; decisão do dono = «pré-screen seguro»

## Problema

A cache global (ADR-0024) reutiliza um veredicto só para o `Cargo.lock`
**idêntico** — logo o efeito-de-rede entre projectos **diferentes** era nulo. A
tentação seria decompor o veredicto por `crate@versão` e, se todos os crates
estivessem «limpos», dar `Approved` sem correr o gate. **Isso seria inseguro.**

### Análise de soundness

- Os achados do `cargo-audit` (RUSTSEC: vulnerability/unmaintained/yanked) são
  propriedade de um `crate@versão` sob um dado snapshot da base — **independentes
  do resto do grafo**. «Este crate está bloqueado» transfere-se de forma **sã**
  entre projectos.
- O `cargo-deny` tem verificações **ao nível do grafo** (versões duplicadas /
  `multiple-versions`, wildcards) que **não** decompõem por crate. Um atalho
  «todos os crates limpos → Approved» **saltaria** essas verificações →
  enfraqueceria o gate (proibido pelo ADR-0005).

## Decisão (pré-screen seguro)

Cachear, por snapshot da base, os `crate@versão` **conhecidos como bloqueados**
(apenas categorias de `cargo-audit`, per-crate-sãs) e usar isso só para
**acelerar um BLOCK**:

1. Após um gate completo, `prescreen::record` grava os achados per-crate-sãos em
   `~/.dx/advisories/<snapshot>.json` (override `DX_ADVISORY_DIR`). **«Limpo»
   nunca é gravado.**
2. Antes de correr o gate completo, `prescreen::screen` lê o `Cargo.lock` actual
   e, se **algum** `crate@versão` já é conhecido como bloqueado sob este snapshot
   **e** a política actual o bloqueia → devolve `Blocked` de imediato, sem correr
   `cargo-audit`/`cargo-deny`.

**Invariante:** o pré-screen só pode produzir `Blocked`. **Nunca** devolve
`Approved`. O caminho `Approved` corre **sempre** o gate completo (audit + deny,
incluindo as verificações de nível-grafo). Um bloqueio de advisory por crate é
sempre um bloqueio — logo acelerá-lo é são.

### Restrições que garantem a soundness

- Só categorias de `cargo-audit` entram na cache (`Vulnerability`,
  `Unmaintained`, `Yanked`, `Malware`). Categorias do `cargo-deny`
  (`License`/`Ban`/`SourceNotAllowed`) são **excluídas** — podem depender do
  grafo.
- A decisão de bloqueio é **reavaliada com a política actual** no momento do
  screen (ex.: `unmaintained` só bloqueia se a política bloquear essa categoria).
- O match exige o **mesmo snapshot** da base (um `crate@versão` bloqueado sob S
  só pré-bloqueia sob S).

## Consequências

- Efeito-de-rede real entre projectos **diferentes**: puxar um crate já conhecido
  como vulnerável bloqueia instantaneamente, sem re-auditar. `dx audit`/`add`/
  `run` reportam «fast-blocked by per-crate pre-screen».
- O gate não é enfraquecido em caso nenhum: nada de novo pode ser aprovado por
  esta via.

## Anti-objectivos

- Nunca dar `Approved` a partir do pré-screen.
- Nunca cachear «limpo» nem categorias de nível-grafo.
- Nunca usar um bloqueio de um snapshot para pré-bloquear sob outro snapshot.
