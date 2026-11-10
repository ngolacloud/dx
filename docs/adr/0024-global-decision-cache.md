# ADR-0024 — Cache de decisões global e à prova de adulteração

- Estado: Aceite (implementado em `feat/trust-layer-overhead-zero`)
- Data: 2026-11-10
- Contexto: plano de melhorias, Fase 4 — a alavanca de rede estilo UV

## Problema

O veredicto do gate era guardado **por projecto** (`<root>/.dx/security/`), pelo
que cada checkout recalculava do zero: worktrees, runners de CI e colegas de
equipa corriam `cargo-audit`/`cargo-deny` repetidamente para o **mesmo** grafo.
O UV ganha tracção porque a sua cache é global e quase-grátis; faltava ao DX o
equivalente para o eixo de confiança.

## Decisão

Adicionar uma **cache global** em `~/.dx/verdicts/` (override `DX_VERDICT_DIR`),
indexada pelo **mesmo** `CacheKey` (lockfile-sha + política + snapshot da base +
versão do scanner). O `run_gate` passa a consultar por ordem:

1. **cache local** (`<root>/.dx/security/`) — a mais rápida;
2. **cache global** (`~/.dx/verdicts/`) — partilhada, assinada; um hit também
   **aquece a cache local**;
3. **calcular** (cargo-audit + cargo-deny) — e publicar em **ambas** as caches.

### Âmbito honesto da reutilização

A chave inclui o sha do `Cargo.lock` **inteiro**. Logo a reutilização exige o
**mesmo** lockfile — o ganho é **CI/equipas/monorepos e worktrees**, não
projectos arbitrários não-relacionados. Decompor o veredicto por `crate@versão`
(para um verdadeiro efeito de rede entre projectos diferentes) é possível mas
mexeria no modelo do gate; fica como evolução futura, para não enfraquecer as
verificações sem desenho cuidado.

### À prova de adulteração (HMAC)

Uma cache partilhada é superfície de ataque: um veredicto `approved` forjado não
pode ser confiado. Cada entrada global guarda um **HMAC-SHA256** sobre
`"<key.digest>|<decision>"`, com uma **chave-segredo local** em
`~/.dx/verdict.key` (32 bytes de `/dev/urandom`, modo 0600, criada uma vez;
override `DX_VERDICT_KEY`). No load, o MAC é recalculado e comparado em tempo
constante; qualquer divergência (chave errada, conteúdo adulterado, decisão
virada) é tratada como **cache miss** e o gate **reavalia** (fail-closed).

- O HMAC autentica a afirmação crítica (**esta chave → esta decisão**). Editar
  os `findings`/espaços em branco não invalida (são detalhe informativo); virar
  a `decision` invalida.
- O ficheiro da chave é a **âncora de confiança**: partilhá-lo numa equipa de
  confiança partilha os veredictos; caso contrário é anti-tamper local à máquina.

## Consequências

- Worktrees/CI/equipas com o mesmo `Cargo.lock` reutilizam o veredicto sem
  recorrer ao scanner. `dx audit`/`add`/`run` reportam se o hit foi **local** ou
  **GLOBAL**.
- A cache global compõe-se com o `dx bundle` (futuro: exportar/importar
  veredictos assinados para on-premises/air-gapped).

## Anti-objectivos

- Nunca confiar num veredicto global sem verificar o HMAC.
- Nunca converter indisponibilidade/adulteração em aprovação (fail-closed).
- Não decompor o gate por crate nesta fase (risco de enfraquecer verificações).
