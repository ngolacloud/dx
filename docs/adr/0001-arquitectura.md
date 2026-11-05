# ADR-0001 — Arquitectura: CLI fina sobre bibliotecas testáveis

- Estado: Aceite
- Data: 2026-11-05
- Contexto do contrato: §2 (arquitectura e compatibilidade)

## Decisão

O DX é uma CLI fina (`dx-cli`, executável `dx`) assente em bibliotecas puras e
testáveis. As responsabilidades são separadas por crate **apenas quando há
benefício real** (não um crate por conceito):

| Crate         | Responsabilidade                                               | I/O |
|---------------|----------------------------------------------------------------|-----|
| `dx-core`     | Modelos de domínio: `Source`, `DepSpec`, `PackageId`, `Plan`.  | Nenhum |
| `dx-manifest` | Ler/escrever Cargo.toml (preservando formato), journal, lock.  | FS |
| `dx-cli`      | Parsing de argumentos, orquestração, delegação no Cargo.       | FS/proc |

Crates previstos (incrementos seguintes), a criar **quando** forem precisos:
`dx-security` (gate), `dx-cargo` (wrapper de resolução por staging),
`dx-report` (relatórios), `dx-build` (perfis LLVM/PGO), `dx-seiva` (adaptador).

## Porquê

- A lógica de domínio (identidade de package, idempotência, plano) testa-se sem
  tocar em disco nem rede — a maioria dos testes do inc.1 são de domínio puro.
- A CLI não contém regras; traduz argumentos em chamadas às bibliotecas. Isto
  permite reutilizar o motor em CI, no adaptador Seiva e em testes de ponta a
  ponta sem passar por `std::process`.

## Compatibilidade com Cargo (inegociável)

- Cargo/rustc continuam a resolver e compilar. **Não** reimplementamos o solver
  nem o compilador. Usamos comandos públicos e formatos documentados.
- Cargo.toml e Cargo.lock permanecem autoritativos. O DX **não** cria um segundo
  lockfile de dependências. `dx.toml` (futuro) só configura tarefas, políticas e
  relatórios.
- Subprocessos são invocados com argv estruturado, nunca por interpolação em
  shell (ver `cargo_passthrough`). stdout estruturado fica separado de progresso
  em stderr.

## Alternativas consideradas

- **Monólito num só crate**: rejeitado — mistura I/O com domínio e dificulta os
  testes de unidade determinísticos.
- **Muitos crates desde já** (`dx-cache`, `dx-net`, …): rejeitado no inc.1 por
  YAGNI; criam-se quando o código correspondente existir.

## Limites

Invocar o Cargo directamente contorna qualquer política do DX (documentado em
`docs/security.md`). O carácter opcional do DX é verdadeiro por desenho.
