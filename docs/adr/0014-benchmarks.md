# ADR-0014 — Harness de benchmarks e orçamento de overhead

- Estado: Aceite (implementado; crate `dx-bench`, comando `dx bench`)
- Data: 2026-11-05
- Contexto do contrato: §9

## Decisão

`dx bench` mede o pipeline de dependências sobre um **mock registry
determinista** (N crates por `path`, sem rede), separando **resolução** (cargo)
de **análise/gate** (DX), com cache de veredicto **frio** (miss) e **quente**
(hit), p50/p95 e um **orçamento de overhead**.

## Baseline medido (este host; 7 reps)

| pkgs | resolve p50/p95 (ms) | gate-frio p50/p95 (ms) | gate-quente p50/p95 (ms) | overhead quente |
|------|----------------------|------------------------|--------------------------|-----------------|
| 10   | 52.6 / 58.2          | 234.6 / 310.0          | 0.2 / 0.2                | 0.3%            |
| 100  | 48.5 / 51.7          | 245.1 / 279.0          | 0.9 / 1.0                | 1.9%            |

Overhead quente = gate-quente p50 / resolução p50.

## Achado e optimização (honesta)

A primeira medição deu overhead quente de **126–144%**: mesmo com cache de
veredicto, `run_gate` re-sondava as ferramentas (`cargo audit --version`,
`cargo deny --version`, `git rev-parse`) ao compor a chave do cache, a cada
chamada — três spawns de processo. **Memoizámos** essas sondas invariantes no
processo (`OnceLock`/mapa por caminho). Resultado: overhead quente **≤2%**.

Isto **não desliga nenhuma verificação** (contrato §9): o gate frio continua a
correr o scan completo (cargo-audit + cargo-deny, ~235–245 ms); a validade da
base e as regras do cache de decisão são iguais. Só evitámos re-sondar versões
invariantes.

## Orçamento proposto

- **Cache quente**: meta ≤10% sobre a resolução — **cumprida** (~0.3–1.9%),
  como objectivo de engenharia (não garantia).
- **Cache frio**: o custo é dominado pelo scan dos scanners externos
  (~235–245 ms), independente do N (grafo por path sem advisories). É o preço
  honesto de uma avaliação de segurança completa.

## Cobertura e limites honestos

- **Implementado**: resolução vs gate, cache frio/quente, p50/p95, escala
  10/100/1000 (mock determinista).
- **FORA deste harness** (reportado em runtime): fase de **download** real,
  **rede rápida/lenta**, **amostra real controlada** (o grafo por path não
  descarrega), memória por fase, grafo com vulnerabilidades vs limpo (o mock é
  limpo). Estes exigem um mock registry HTTP + amostra real — trabalho futuro.

## Reprodução

```console
$ dx bench --sizes 10,100 --reps 7        # 1000 também suportado (mais lento)
```
Determinístico: `dx-bench::gen_mock_workspace` gera o mesmo grafo para o mesmo N.
