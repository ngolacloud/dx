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
- **Nome publicado (crate)**: **não** `dx` (ocupado). O package de publicação é
  `dx-cli`, com `[[bin]] name = "dx"`. Nome de publicação alternativo
  (ex.: `ngola-dx`) fica em aberto até o dono decidir publicar.

Não substituímos silenciosamente nenhuma ferramenta existente no PATH; se um
`dx` de terceiros for detectado na instalação, o instalador (futuro) avisa em
vez de sobrescrever.

## Consequências

- Nada é publicado nesta fase. Qualquer afirmação de disponibilidade de nome de
  publicação seria falsa e está explicitamente evitada.
- A documentação refere sempre o executável como `dx` e o crate como `dx-cli`.
