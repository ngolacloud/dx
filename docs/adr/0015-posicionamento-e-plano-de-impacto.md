# ADR-0015 — Posicionamento do produto e plano de impacto

- Estado: Aceite (directriz do dono; `dx add --plan`, `dx why`, `dx explain` implementados)
- Data: 2026-11-05
- Contexto: avaliação de produto do dono (engenharia sénior de package managers)

## Posicionamento

**Actualizado pelo [ADR-0020](0020-camada-de-confianca-nao-substituto.md):** o
DX é a _camada de confiança do Cargo_, não um substituto — corre _com_ o Cargo.
O eixo de valor é confiança unificada (supply-chain/segurança/release), nunca
velocidade. A frase abaixo mantém-se válida como descrição das garantias.

> **DX — gestão de dependências Rust com alterações explicáveis, segurança
> antes da execução e recuperação previsível, compatível com Cargo.**

O Cargo já é um gestor de pacotes sólido; `cargo-deny`/`cargo-vet`/`cargo-dist`/
`cargo-binstall`/`cargo-auditable` resolvem partes. A lacuna é a **fragmentação
da experiência e das garantias** — hipótese de produto, não exclusividade. O DX
**reutiliza** a semântica Cargo e **compõe** estas ferramentas, concentrando-se
em três provas:

1. **Adicionar com impacto explicado** (`dx add --plan`) — feito.
2. **Corrigir uma vulnerabilidade transitiva com alteração mínima testada**
   (`dx update --security`) — próximo (P1).
3. **Recuperar de uma instalação interrompida sem danificar o projecto** — já
   garantido pelo journal/recuperação (ADR-0004); a demonstrar como prova.

Um utilizador de Axum/Bevy/CLI deve ter motivos para o DX **sem conhecer Seiva**
(Seiva/LLVM/release são P2, completam o ecossistema).

## `dx add --plan` (P0 — "o que estou a introduzir?")

Resolve o grafo candidato em staging (sem aterrar, sem compilar, sem `build.rs`)
e mostra:

- **packages novos** (directos/transitivos) — diff do lockfile candidato vs actual;
- **superfícies de EXECUÇÃO** durante o build: `build.rs`, proc-macros, `links`
  nativos (do `cargo metadata`) — a superfície de segurança que o `build.rs`
  representa;
- **MSRV** exigido pelos novos packages (max `rust-version`);
- **segurança**: veredicto do gate (cargo-audit + cargo-deny) sobre o grafo
  candidato, com decisão approved/blocked/pending;
- **download**: packages já em cache vs novos (sem inventar bytes);
- **mudança no manifesto**.

Custo de compilação/tamanho só apareceria como **medição identificada**, nunca
número inventado.

## `dx why` / `dx explain` (P0 — explicar o grafo)

- `dx why <crate>` — árvore inversa (`cargo tree -i`): quem puxa o crate.
- `dx explain --duplicates` — versões múltiplas (`cargo tree -d`).
- `dx explain --feature <f>` — quem activa a feature (`cargo tree -e features -i`).

As respostas são **confirmadas pela resolução Cargo**. Remover uma feature pode
alterar comportamento/targets — o DX explica, não decide pelo utilizador.

## Prioridades (do dono)

| Prioridade | Entrega | Estado |
|---|---|---|
| P0 | `add --plan`, compat Cargo, alterações recuperáveis | add --plan ✅; recuperação ✅ |
| P0 | segurança obrigatória + cache de decisões + explicação de bloqueios | ✅ (gate no `add`) |
| P0 | `why/explain` deps/features/origens | ✅ |
| P1 | actualizações de segurança validadas por testes | **próximo** (`dx update`) |
| P1 | build isolado + ferramentas fixadas por projecto | isolado ✅; `dx tool` por fazer |
| P1 | pacotes offline verificáveis + diagnóstico de ambiente | `dx fetch`/SBOM ✅; `dx environment` por fazer |
| P2 | Seiva/admin, LLVM, release prepare | ✅ |

## Limites honestos

- Impacto completo exige manifesto auto-contido (staging); workspace/path →
  mostra a mudança de manifesto e remete para `dx audit` (estado parcial).
- "Features e quem as activa" é parcial no `--plan` (mostra `dx explain --feature`
  para a árvore completa).
- `dx update`, `dx tool`, `dx environment` ainda não implementados.
