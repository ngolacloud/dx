# ADR-0016 — `dx update`: explainable, security-driven, recoverable

- Estado: Aceite (implementado)
- Data: 2026-11-05
- Contexto: directriz de produto do dono (P1, prova pública nº2)

## Decisão

`dx update` resolve um grafo **candidato** num staging isolado (via
`cargo update`, lockfile real intacto) e opera em três modos:

- `--plan` — mostra as mudanças de versão (old -> new) que o update faria.
- `--security` — corre o gate **antes** (grafo actual) e **depois** (grafo
  actualizado); reporta que advisories o update **CLEARS** e quais **permanecem
  sem correcção compatível**. Para estes, o DX **explica** o bloqueio e **não**
  troca a biblioteca por outra (API/comportamento diferentes exigem revisão
  própria).
- `--apply` — escreve o lockfile actualizado de forma transaccional (journal),
  **valida** com o gate (e `--test` opcional = `cargo test`) e, se a validação
  falhar, **restaura** o lockfile anterior (recuperação). Projecto nunca fica
  num estado por validar.

`name` opcional restringe ao update de um package (alteração mínima).

## Prova pública nº2 (ao vivo)

```
smallvec fixado em 1.6.0 (RUSTSEC-2021-0003, buffer overflow)
dx update smallvec --security
  Version changes (1): smallvec: 1.6.0 -> 1.16.4
  cleared (1): smallvec@1.6.0 (vulnerability) — Buffer overflow in SmallVec::insert_many
  remaining blockers: none
dx update smallvec --apply
  Applying updated lockfile... ✓ Applied and validated (gate approved).
  final: smallvec 1.16.4
```

Corrigir uma vulnerabilidade transitiva com a **menor** alteração compatível,
**validada** e **reversível** — sem desligar verificações nem trocar de
biblioteca.

## Honestidade

- **Testes verdes não provam ausência de regressões**: `--apply --test` corre a
  suite e reporta; a validação não é prova de correcção total.
- Sem correcção compatível ⇒ o bloqueio é **explicado**, não contornado.
- Staging exige manifesto auto-contido (como o `add`/`plan`); workspace/path →
  `NotApplicable` reportado.
- `--apply` valida com o gate do DX; a decisão APROVADO depende da validade da
  base (ver ADR-0005) — base expirada mantém o projecto por validar e restaura.

## Prova

- Unit (`dx-cli::updatecmd`): diff de versões (bump detectado; vazio quando
  igual). Ao vivo: fluxo security+apply acima; recuperação restaura o lockfile
  anterior em caso de validação falhada (caminho de código exercitado).
