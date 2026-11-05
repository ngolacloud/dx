# ADR-0011 — `dx release prepare` (prepara ≠ publica)

- Estado: Aceite (implementado; crate `dx-release`)
- Data: 2026-11-05
- Contexto do contrato: §8 (parte de release)

## Decisão

`dx release prepare` valida em ambiente limpo e produz **artefactos para
revisão**, **sem** publicar, criar tags ou mudar versões.

### Validações (falha ⇒ não produz artefactos)

1. `fmt --check` · 2. `clippy --all-targets -D warnings` · 3. `cargo test` ·
4. `cargo build --release` (compila em limpo) · 5. **gate de segurança DX**
(inclui "refresh obrigatório antes de release": base expirada ⇒ não pronto) ·
6. compatibilidade de API (`cargo-semver-checks` se presente; senão **"sem
cobertura"**, reportado, não fingido).

### Artefactos (`target/dx-release/`)

- **SBOM** CycloneDX 1.5 a partir do Cargo.lock: cada package com `purl`
  (`pkg:cargo/...`) e hash sha256 (registry). Proveniência verificável.
- **proveniência** (`provenance.json`): tool, timestamp, versão do rustc, target
  do host, commit git e os artefactos com sha256+tamanho.
- **ordem de publicação** topológica (dos membros do workspace, via
  `cargo metadata`): dependências primeiro, determinística.
- **binários + checksums**: alvos `bin` do metadata construídos em release.

### Prepara ≠ publica · idempotente

Não cria tags, não muda versões, não publica. Repetir reescreve os mesmos
ficheiros em `target/dx-release/` — sem duplicar nada. Publicar exige instrução
específica (não implementado; seria um comando separado e explícito).

## Prova

- Unit (`dx-release`, 5 testes): topo sort (deps-primeiro, detecção de ciclo,
  parse de metadata), SBOM CycloneDX, bundle+checksum roundtrip.
- Ao vivo: base expirada ⇒ **NÃO PRONTO** (segurança pending, §5); com janela
  válida ⇒ **PRONTO**, ordem `…→dx-cli` (dx-cli último), SBOM com hashes,
  proveniência com commit/rustc/host, binário `dx` (2,85 MiB) + sha256.

## Limites honestos

- `cargo-semver-checks` não instalado ⇒ compatibilidade de API fica sem
  cobertura (reportado).
- Cross-compilação para múltiplos targets exige as toolchains instaladas; hoje
  produz o target do host.
- SBOM cobre o grafo do lockfile; git/path sem checksum de registry entram sem
  hash (coerente com a honestidade de cobertura de `dx-integrity`).
