# ADR-0006 — Gate ligado ao `add` com staging isolado

- Estado: Aceite (implementado; cobre manifestos auto-contidos)
- Data: 2026-11-05
- Contexto do contrato: §5.1 (resolver em staging sem executar código), §5.5/§5.6

## Decisão

`dx add` deixou de ser uma pura edição de texto: antes de aterrar uma
dependência, o DX **resolve o grafo candidato num staging isolado** e corre o
gate de segurança. A ordem é **resolver → auditar → só então confirmar**.

### Fluxo

1. Planear a adição (idempotência/conflito de origem como no ADR-0002). No-op e
   `--dry-run` retornam **antes** do gate (nada novo entra no grafo).
2. Mutar o manifesto **em memória** (ainda não escrito em disco).
3. Montar um directório temporário com o Cargo.toml mutado (+ `src/` mínimo e o
   Cargo.lock existente como ponto de partida) e correr
   `cargo generate-lockfile`. Isto resolve a partir do índice e escreve um
   lockfile candidato **sem compilar nem executar `build.rs`/proc-macros/testes/
   exemplos/hooks**.
4. Correr o gate (`cargo-audit`) sobre o lockfile candidato.
5. Decidir:
   - **Approved** → escrever o manifesto real (transaccional, ADR-0004).
   - **Blocked** (exit 4) → manifesto **inalterado**; mostrar achados + caminho
     de dependências.
   - **Pending** (exit 5, base expirada/indisponível) → manifesto **inalterado**;
     o download/resolução pode ter ocorrido, o **uso** não é autorizado.

O cache de decisão é guardado sob o projecto real (chave inclui o lockfile
candidato), pelo que repetir um `add` idêntico reutiliza o veredicto.

## Porquê staging isolado

O manifesto real nunca é tocado antes da decisão. Bytes/lockfile podem ser
produzidos no staging, mas **nada aterra sem aprovação** — a propriedade central
do §5. `generate-lockfile` é o ponto exacto onde se obtém o grafo sem correr
código de dependências.

## Limite honesto (âmbito desta versão)

Cobre manifestos **auto-contidos** (o que o `dx new` gera). Para `[workspace]`,
herança de workspace (`.workspace = true`) ou dependências `path` — onde a cópia
isolada quebraria a resolução — o staging devolve **NotApplicable**: a edição de
texto aterra, mas o resultado é marcado **não-verificado** (exit 6) com
instrução de correr `dx audit`. **Não** fingimos gate onde ele não correu.

Resolução falhada (ex.: `--offline` sem cache, git inexistente, versão
inexistente) ⇒ manifesto inalterado, exit 1.

Validade da base configurável por `DX_MAX_DB_AGE_SECS` (o contrato pede validade
máxima configurável); baseline 24 h.

## Prova

- Unit (`dx-cli::stage`): aplicabilidade (workspace/herança/path → NotApplicable).
- Integração offline: idempotência/git/recuperação via NotApplicable; gate real
  (pending/approved) atrás de `DX_GATE_TESTS=1` com serde do cache.
- Ao vivo: `dx add serde@1 --offline` → PENDENTE (base 167h, não aterra, exit 5);
  com `DX_MAX_DB_AGE_SECS` alargado → APROVADO (aterra, exit 0).

## Ainda em falta (próximo)

Isolamento de build de dependências (rede desligada após fetch, FS mínimo,
limites), extracção segura, integridade própria, `cargo-deny` e política de
origem anti-confusion continuam planeados (ver threat model e progress.md).
