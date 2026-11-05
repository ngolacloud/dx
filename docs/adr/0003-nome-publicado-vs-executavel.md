# ADR-0003 — Nome publicado vs nome do executável

- Estado: Aceite
- Data: 2026-11-05
- Contexto do contrato: §1.3 (disponibilidade de nomes, sem substituir ferramentas)

## Facto apurado (2026-11-05)

- `which -a dx` → **nada**: o executável `dx` está livre no PATH deste host.
- `GET crates.io/api/v1/crates/dx` → o crate **`dx` já existe** em crates.io
  ("A subset of MDX"), versão `0.0.0`, `yanked: true`, criado 2021-04-06.
- `dxctl` já existe (ferramenta distinta, do `delonix-paas`). `dx` ≠ `dxctl`.

## Decisão

- **Executável**: `dx` (marca mantida).
- **Nome publicado (crate)**: **por decidir**. Internamente o package da CLI é
  `dx-cli` com `[[bin]] name = "dx"`, mas para crates.io é preciso um prefixo
  livre (ver "Actualização" abaixo).

## Actualização (2026-11-05) — crates.io

Verificação ao vivo: `dx`, **`dx-cli`** (v0.3.1, 2024) e **`dx-core`** (v0.3.0)
estão **TODOS ocupados** por crates de terceiros. Logo, publicar o workspace
como está é **impossível** — os 13 crates (`dx-core`…`dx-cli`) precisariam de um
prefixo. Prefixos livres confirmados: `ngoladx-*` e `dxkit*`.

Opções consideradas:
- **A — prefixo** (`ngoladx-core`…`ngoladx-cli`, binário continua `dx`):
  publica no crates.io, reclama 13 nomes.
- **B — só GitHub**: distribuir via `cargo install --git https://github.com/ngolacloud/dx`
  (sem reclamar nomes).

## Decisão final (2026-11-05) — opção A

O dono escolheu a **opção A**. Os 13 crates publicam-se com o prefixo livre
**`ngoladx-`** (`ngoladx-core`, `ngoladx-manifest`, …, `ngoladx-cli`). Os 13
nomes foram confirmados livres no crates.io antes do rename.

Mecânica, sem churn de código:
- Cada crate tem `[package] name = "ngoladx-X"` **e** `[lib] name = "dx_X"`,
  pelo que o nome de importação em Rust continua `dx_X`.
- `[workspace.dependencies]` mantém as chaves `dx-X` com
  `package = "ngoladx-X"`, pelo que os dependentes continuam a escrever
  `dx-core.workspace = true` / `use dx_core::…` sem alterações.
- `ngoladx-cli` mantém `[[bin]] name = "dx"` — o executável continua `dx`.

Instalação a partir do crates.io: `cargo install ngoladx-cli` (binário `dx`).

GitHub público: <https://github.com/ngolacloud/dx>.

Não substituímos silenciosamente nenhuma ferramenta existente no PATH; se um
`dx` de terceiros for detectado na instalação, o instalador (futuro) avisa em
vez de sobrescrever.

## Consequências

- Publicável no crates.io sob `ngoladx-*`; o executável e os nomes de
  importação permanecem `dx`/`dx_*`.
- A documentação refere o executável como `dx`, o crate da CLI como
  `ngoladx-cli` e os nomes de importação como `dx_*`.
