# ADR-0007 — cargo-deny: origens, licenças e bans

- Estado: Aceite (implementado; complementa o cargo-audit)
- Data: 2026-11-05
- Contexto do contrato: §5 ("cargo-deny para origens, licenças e dependências";
  "Defesa contra crates maliciosas" → política de origem anti dependency-confusion)

## Decisão

O gate passou a correr também o **`cargo-deny`** para `licenses`, `sources` e
`bans`, complementando o `cargo-audit` (advisories). Reutilizamos a ferramenta
madura em vez de reimplementar verificação de licenças/origens.

- `cargo-deny --format json` emite NDJSON; mapeamos cada diagnóstico de
  severidade `error`/`warning` para um `Finding` categorizado. Os advisories
  ficam **fora** do `cargo-deny` (só o `cargo-audit` os trata), evitando dupla
  contagem.
- Categorias novas: `license` e `ban` (acção **Warn** por omissão, configurável
  para Block — são específicas da organização). Violações de origem mapeiam para
  `source-not-allowed`, que é **bloqueio duro** (não enfraquecível).

## Política de origem (anti dependency-confusion)

O `deny.toml` embutido (`assets/default-deny.toml`, usado quando o projecto não
traz o seu) recusa **registries desconhecidos** (`unknown-registry = "deny"`,
`allow-registry` só crates.io): um package só vem de uma origem explicitamente
permitida — é isto que evita o fallback silencioso de um package privado para
crates.io. Dependências git são explícitas no manifesto, logo não recusadas por
omissão. O `deny.toml` do projecto tem precedência sobre o embutido.

## Honestidade de cobertura

Se o `cargo-deny` não estiver instalado, o gate **não falha**: corre só o
`cargo-audit` e declara que licenças/origens/bans ficam **sem cobertura**
(linha "Scanners" e `dx doctor`). Nunca fingimos uma verificação que não correu.
A presença/ausência do `cargo-deny` entra na chave do cache de decisão.

## Prova

- Unit (`dx-security::deny`): parse NDJSON (licença+origem), ignora help/note,
  config embutida presente.
- Ao vivo: `dx audit` com `deny.toml` que não permite `Unicode-3.0` →
  `unicode-ident` sinalizado (`deny:rejected`, categoria `license`, Warn);
  `dx doctor` mostra `cargo-deny: presente`.

## Limites

- Licença/ban são Warn por omissão: o bloqueio é decisão de política da
  organização (configurável), não imposto universalmente.
- A verificação de origem depende do `allow-registry`/`allow-git` configurados;
  um registry privado deve ser declarado para não ser recusado.
