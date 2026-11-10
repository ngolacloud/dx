# ADR-0023 — `dx run <script.rs>`: scripts de ficheiro único auditados

- Estado: Aceite (implementado em `feat/trust-layer-overhead-zero`)
- Data: 2026-11-10
- Contexto: plano de melhorias, Fase 3 — ergonomia diária análoga a PEP 723/`uvx`

## Problema

O UV criou hábito diário com `uv run`/`uvx` (PEP 723): correr um ficheiro único
com dependências declaradas *inline*, em ambiente efémero. O DX só tinha razões
para ser aberto ao **adicionar dependências**; faltava um motivo para o usar
todos os dias. O Rust tem o `cargo -Zscript` (nightly) para o mesmo fim, mas
**não audita** o grafo antes de correr.

## Decisão

Adicionar `dx run <file.rs>`: um ficheiro `.rs` com um **frontmatter** opcional
(estilo `cargo -Zscript`, cercado por `---`) que declara as dependências:

```rust
#!/usr/bin/env dx
---
[dependencies]
serde = "1"
---
fn main() { /* ... */ }
```

O **diferenciador do DX** face ao `cargo -Zscript`/`uvx`: o grafo do script
passa pelo **mesmo gate de segurança** do `dx add` **antes** de executar.

### Fluxo

1. Partir o frontmatter do código (crate `dx-script`, puro).
2. Sintetizar um `Cargo.toml` **efémero** num temp dir: `[package]` (nome do
   stem do ficheiro, se omisso) + as deps do frontmatter + `[workspace]` vazio
   (raiz de workspace isolada — nunca absorvida por um workspace pai).
3. `cargo generate-lockfile` (resolve **sem** compilar nem correr `build.rs`).
4. `run_gate` sobre esse lockfile exacto (o mesmo do `dx add`).
5. Veredicto:
   - **Approved** → `cargo run` no pacote efémero, reencaminhando os args do
     utilizador (`dx run s.rs -- a b` ou `dx run s.rs a b`).
   - **Blocked** → **não executa** (exit 4).
   - **Pending** (base de advisories expirada) → **não executa** (exit 5).

### Detecção

`dx run` continua a delegar no `cargo run` para projectos normais. Só entra em
modo-script quando o primeiro argumento **termina em `.rs` e é um ficheiro
existente**. Assim não há ambiguidade nem regressão do comportamento anterior.

### Porquê `status()` e não `exec()`

Ao contrário do passthrough overhead-≈0 (ADR-0021), aqui corremos o `cargo run`
com `status()` (não `exec`) para que o `TempDir` do pacote efémero seja limpo no
fim. O custo de um processo extra é irrelevante para um script.

## Consequências

- O DX ganha a ergonomia diária do `uv run`/`uvx`, mas com a garantia que o
  `cargo -Zscript` não dá: **auditado antes de executar**.
- Scripts são efémeros e isolados; não tocam em nenhum projecto.
- Reusa integralmente o gate (`dx-security`) e o relatório de veredicto.

## Anti-objectivos

- Não inventar um formato de frontmatter próprio — segue a convenção `---` do
  `cargo -Zscript` (incl. infostring `---cargo`), para portabilidade.
- Não executar nunca um script com veredicto `Blocked`/`Pending`.
- Não reimplementar resolução/compilação: `cargo` resolve e corre.
