# ADR-0010 — Perfis de build LLVM e PGO

- Estado: Aceite (implementado; crate `dx-build`)
- Data: 2026-11-05
- Contexto do contrato: §7 (Builds e LLVM)

## Decisão

Crate `dx-build` define perfis de build e mede-os, usando o **backend LLVM da
toolchain Rust** (não construímos LLVM). Os perfis são aplicados por
`cargo --config` — **sem** tocar no Cargo.toml do utilizador (que continua
autoritativo).

### Perfis (nunca lto=fat/cg=1 como default)

| Perfil | opt-level | LTO | codegen-units | strip | Uso |
|--------|-----------|-----|---------------|-------|-----|
| `dev` | 0 | — | (default) | não | iteração |
| `release` | 3 | **thin** | 16 | não | **default equilibrado** |
| `throughput` | 3 | fat | 1 | não | **OPT-IN**; lento a compilar |
| `size` | z | thin | 1 | symbols | mínimo tamanho |

`throughput` (fat-LTO+cg=1) é opt-in e documentado como lento — **não** é o
default (§7). Nenhum perfil mexe em `panic`/`overflow-checks`/segurança só para
ganhar números. `release` preserva símbolos (sem strip) para diagnóstico.

### target-cpu=native

`--native` adiciona `-Ctarget-cpu=native` com **aviso**: o binário só corre na
microarquitectura do host; **não** para releases distribuídas (§7).

### Medição (`dx build --measure`, `dx bench-build`)

Tempo de relógio (`std::time::Instant`), **pico de memória** (GNU `/usr/bin/time
-v`, campo "Maximum resident set size"; `None` se ausente — não inventamos) e
**tamanho do binário**. `bench-build` limpa antes de cada perfil (build limpo) e
tabula. Medido ao vivo: dev 0,11 s / 4,16 MiB; release 2,03 s / 0,44 MiB; size
1,80 s / 0,31 MiB.

### PGO (`dx pgo`)

Pipeline real: (1) `-Cprofile-generate`; (2) correr workload (o binário ou um
comando dado) → `.profraw`; (3) `llvm-profdata merge`; (4) `-Cprofile-use`
(+`-pgo-warn-missing-function`); (5) comparar.

**Compatibilidade de versão é imposta, não assumida**: `profdata_tool()`
prefere o `llvm-profdata` da **sysroot do rustc** (versão garantidamente igual)
antes do `/usr/bin`. Na prova ao vivo, o `/usr/bin/llvm-profdata` (formato v9)
**recusou** o perfil do rustc 1.96 (v10) com erro claro — exactamente o reporte
honesto que o §7 exige; com o `llvm-tools-preview` da toolchain, o pipeline
completou (instrumentado 1,29 MiB → com perfil 0,42 MiB).

## Limites honestos

- O ganho de **performance** do PGO só se prova medindo o serviço com carga
  representativa (inc.3/§9) — `dx pgo` reporta tamanhos, não throughput.
- Cache de compilação (chave toolchain/target/flags/fontes/deps) e isolamento
  de artefactos entre tenants de CI: ainda planeados.
