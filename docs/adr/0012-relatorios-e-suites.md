# ADR-0012 — Relatórios de teste e suites configuráveis

- Estado: Aceite (implementado; crate `dx-report`)
- Data: 2026-11-05
- Contexto do contrato: §8 (testes, relatórios)

## Decisão

`dx test --suite <nome>` corre uma suite e emite um relatório num de quatro
formatos. O modelo de relatório (`dx-report`) é independente do runner.

### Suites configuráveis (não por nome sem config)

- Builtins mapeados a Cargo: `unit` (`--lib --bins`), `doc` (`--doc`),
  `integration` (`--tests`).
- Qualquer outra suite tem de estar em `dx.toml` (`[test.suites.<nome>]` com
  `runner`, `kind`). Nome desconhecido sem config ⇒ **recusa** (§8).
- `kind`: `rust` (parse libtest por-teste), `command` (sucesso/falha por exit
  code), `load` (falha por thresholds).

### Formatos

- `terminal` (com/sem ANSI via `--no-ansi`), `json` (**versionado**:
  `schema_version`), `junit` (XML), `html` (auto-contido, sem recursos externos).
- `--out <ficheiro>` escreve; senão stdout.

### Invariantes do §8

- **Erro de runner nunca vira suite verde**: `Suite.ok` exige runner bem
  terminado **e** zero falhas; falha de compilação sem casos ⇒ `runner_error` +
  não-verde.
- **HTML escapado** e **segredos removidos** (`scrub`) em TODOS os emissores,
  incluindo o JSON (também é um relatório).
- **Carga falha por thresholds** (p95/p99/erros/rps), não só por exit code: o
  gerador escreve métricas JSON (`DX_LOAD_METRICS` ou
  `target/dx-load-metrics.json`) e o DX avalia-as contra `[test.suites.X.load]`.

## Prova

- Unit (`dx-report`, 12 testes): scrub de segredos, escape HTML, JSON
  versionado sem segredos, JUnit com failure escapado, parse libtest
  (ok/FAILED/ignored + `--report-time`), resolução de suites (builtin/dx.toml/
  recusa), avaliação de thresholds.
- Ao vivo: `dx test --suite unit` (terminal/JUnit/JSON), suite `command` via
  dx.toml, recusa de suite sem config, teste a falhar → exit 1, threshold p95
  excedido → FALHA.

## Limites honestos

- **Flaky** existe no modelo mas não é detectado sem um runner com retry
  (nextest); hoje não o sintetizamos.
- Mensagens de falha detalhadas do libtest (blocos "---- stdout ----") não são
  anexadas por caso nesta versão (estado por-teste sim).
- `nextest` não está instalado; a sua instalação passaria pela política de
  segurança (contrato §8) — não auto-instalado.
