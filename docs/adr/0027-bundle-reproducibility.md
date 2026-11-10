# ADR-0027 — Reprodutibilidade do `dx bundle` (fontes vendored)

- Estado: Aceite (implementado em `feat/trust-layer-overhead-zero`)
- Data: 2026-11-10
- Contexto: item adiado do ADR-0025

## Problema

O `dx bundle` (ADR-0019) produzia um pacote offline verificável — o
`dx bundle verify` detecta adulteração recomputando os hashes — mas **não**
provava que o bundle era **reproduzível**: que re-gerar a partir do mesmo
lockfile dá as mesmas fontes de dependências. Sem isso, o `verify` só diz «estes
bytes não mudaram desde que os gravei», não «estes bytes são os que qualquer
pessoa obtém a partir deste lockfile».

## Decisão

Adicionar `dx bundle reproduce <dir>`: re-vendoriza a partir do **próprio
`Cargo.lock` do bundle** para um directório scratch e compara o **agregado do
`vendor/`** com o registado no `bundle.json`.

- As fontes vendored são determinísticas para um dado `Cargo.lock` — os crates
  no crates.io são imutáveis e têm checksum. Logo o agregado reproduz.
- O **timestamp** e o **`environment.json`** são específicos do instante/host e
  são **excluídos** da reivindicação de reprodutibilidade (o `reproduce` só
  compara o `vendor/`).

Âmbito honesto: isto prova que as **fontes de dependências** (o código que de
facto compilarias) reproduzem byte-a-byte; **não** é uma reivindicação de
reprodutibilidade do binário final (que depende de toolchain/flags — cobertos
pelo `dx environment`).

## CI

O `ci.yml` ganha um job (best-effort, depende de rede) que cria um projecto
mínimo, exporta um bundle e corre `dx bundle reproduce`, falhando se o agregado
divergir. Assim, uma regressão no caminho de vendoring/hashing é apanhada.

## Consequências

- `dx bundle` passa a ter três verbos coerentes: `export` (produzir),
  `verify` (integridade — não mudou), `reproduce` (determinismo — qualquer um
  obtém o mesmo a partir do lockfile).
- Fecha o item de reprodutibilidade adiado, com âmbito explícito (fontes, não
  binário).

## Anti-objectivos

- Não reivindicar reprodutibilidade byte-a-byte do **binário** (fora de âmbito;
  é o papel do registo de ambiente + toolchain fixada).
- Não incluir timestamp/ambiente na comparação de reprodutibilidade.
