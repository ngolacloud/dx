# ADR-0002 — Contrato Cargo: identidade de package e tabelas de dependências

- Estado: Aceite
- Data: 2026-11-05
- Contexto do contrato: §2, §4

## Decisão

Um package identifica-se por **nome + versão + origem + revisão/checksum**,
nunca só pelo nome (`dx_core::PackageId`). A intenção declarada pelo utilizador
é `dx_core::DepSpec`, que modela tudo o que Cargo.toml distingue:

- origem: registry (crates.io ou alternativo), git (com `rev`/`tag`/`branch`),
  path;
- `version_req`, `features` (normalizadas: ordenadas, sem duplicados),
  `default-features`, `optional`;
- secção: `dependencies` / `dev-dependencies` / `build-dependencies`;
- `target` (`cfg(...)`/triple);
- rename (`package = "..."`).

## Idempotência (regra central)

`Manifest::plan_add(spec)` devolve:

- `None` — já convergido → **no-op**, sem churn no manifesto/lockfile;
- `Add` — a chave não existe;
- `Update` — existe, **mesma origem**, estado diferente (versão/features);
- `SourceConflict` (erro) — existe com **origem diferente**: mudar de origem é
  sempre explícito, nunca efeito lateral de `add`.

As features são normalizadas antes de comparar, pelo que `--features a,b` e
`--features b,a,a` representam o mesmo estado (testado).

## Preservação de formato

Edição via `toml_edit`: mexemos só na chave visada, pelo que comentários, ordem
e espaçamento do resto do documento ficam intactos (testado).

## Resolução de versão (decisão de ordenação)

`dx add <crate>` de registry **sem** versão explícita é recusado no inc.1 com
orientação (`crate@<req>` ou `--version`). A resolução automática de "latest"
fica para o inc.2 **de propósito**: escolher uma versão nova e aterrá-la sem
passar pelo gate de segurança violaria o contrato §5 (resolver candidato →
auditar → só então confirmar). Git/path com `rev`/caminho são aceites já.

## Alternativas

- **Envolver `cargo add`**: rejeitado como motor principal — tira ao DX a
  autoria da escrita (necessária para plano/journal/idempotência byte-a-byte) e
  a ordenação resolver→auditar→confirmar. `cargo` continua a ser usado para
  resolver/compilar, mas a escrita do manifesto é do DX.
