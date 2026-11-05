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

Decisão de marca pendente do dono. Opções:
- **A — prefixo** (ex.: `ngoladx-core`…`ngoladx-cli`, binário continua `dx`):
  publica no crates.io, reclama ~13 nomes.
- **B — só GitHub**: distribuir via `cargo install --git https://github.com/ngolacloud/dx`
  (funciona hoje, sem reclamar nomes). Recomendado até haver tracção.

GitHub já é público: <https://github.com/ngolacloud/dx>.

Não substituímos silenciosamente nenhuma ferramenta existente no PATH; se um
`dx` de terceiros for detectado na instalação, o instalador (futuro) avisa em
vez de sobrescrever.

## Consequências

- Nada é publicado nesta fase. Qualquer afirmação de disponibilidade de nome de
  publicação seria falsa e está explicitamente evitada.
- A documentação refere sempre o executável como `dx` e o crate como `dx-cli`.
