# DX — relatório de progresso

- Data: 2026-11-05
- Host: Rust 1.96.0, LLVM 22.1.2; `cargo-audit` 0.22.2; `cargo-deny` 0.20.2.
- Repositório: `projects/dx` (git independente, **fora** do workspace Seiva).
- Seiva de referência revalidado: HEAD `aed02d1` (bate com o snapshot do contrato).

## Legenda de estado

**Implementado+testado** · **Implementado** (sem teste automático dedicado) ·
**Experimental** · **Planeado**.

## Incremento 1 — contrato Cargo e mutações idempotentes

| Item | Estado | Evidência |
|------|--------|-----------|
| Modelos de domínio (`Source`/`DepSpec`/`PackageId`/`Plan`), identidade por nome+versão+origem+rev/checksum | Implementado+testado | `dx-core` (4 testes) |
| `dx new`/`dx init` (`--lib`/`--bin`, recusa dir não vazio) | Implementado+testado | `new_lib_scaffolds_files`, `new_refuses_nonempty_dir` |
| `dx add` idempotente (features normalizadas; no-op byte-a-byte) | Implementado+testado | `add_is_idempotent_byte_for_byte`; demo ao vivo (sha256 igual) |
| Conflito de origem é erro (nunca troca silenciosa) | Implementado+testado | `source_conflict_refuses` |
| Registry sem versão recusado com orientação | Implementado+testado | `registry_without_version_is_rejected_with_guidance` |
| Typosquatting → recusa com sugestão | Implementado+testado | `typosquat_is_refused_with_suggestion` |
| `--dry-run` (plano, nada escrito) | Implementado+testado | `dry_run_writes_nothing` |
| Escrita transaccional: journal + `tmp+fsync+rename` + recuperação de crash | Implementado+testado | `transaction_applies_and_recovers`, `pending_journal_is_recovered_on_next_op` |
| Lock de workspace consultivo (fs4) | Implementado | `dx-manifest::lock` |
| `dx remove` idempotente | Implementado+testado | `remove_is_idempotent` |
| Git/path deps | Implementado+testado | `git_dep_with_rev_lands_in_manifest` |
| Passthrough estruturado p/ Cargo (`build`/`check`/`run`/`test`/`fmt`/`lint`) | Implementado | demo (projecto gerado compila) |
| "Cargo sem DX": projecto gerado compila sem o DX | Implementado+testado (ao vivo) | `cargo build` no `demo-api` |

## Incremento 2 — fundação do gate de segurança

| Item | Estado | Evidência |
|------|--------|-----------|
| Política por categoria (vuln/malware/yanked/unmaintained/integrity/source); bloqueios duros não enfraquecíveis; `harden_with` (o mais apertado vence) | Implementado+testado | `dx-security::policy` (2 testes) |
| Validade da base (frescura por commit git HEAD; injecção de `now` p/ teste) | Implementado | `dx-security::db`; demo (167h → inválida) |
| Invocação real do `cargo-audit --no-fetch --json` + normalização robusta por `Value` | Implementado+testado | `dx-security::audit` (2 testes); demo (RUSTSEC-2021-0003) |
| Veredicto approved/blocked/pending; bloqueio vence staleness; limpo+expirado = pending | Implementado+testado | `dx-security::verdict` (5 testes) |
| Cache de decisão (chave = lockfile+política+snapshot+scanner; miss em qualquer mudança) | Implementado+testado | `cache_roundtrip_and_key_sensitivity`; demo (cache hit) |
| Caminho de dependências legível (`cargo tree -i`) | Implementado | demo (smallvec → vuln-demo) |
| `dx audit` / `dx doctor` com códigos de saída coerentes (0/4/5) | Implementado | demo ao vivo |

### Gate ligado ao `dx add` (ADR-0006)

| Item | Estado | Evidência |
|------|--------|-----------|
| Staging isolado: resolve grafo candidato via `cargo generate-lockfile` SEM compilar/executar build.rs | Implementado+testado | `dx-cli::stage` (4 testes); demo ao vivo |
| `add` só aterra se **Approved**; Blocked (4)/Pending (5) não aterram; manifesto intacto | Implementado+testado | `gate_*` (DX_GATE_TESTS); demo (serde pending→não aterra, approved→aterra) |
| Formas fora de âmbito (workspace/herança/path) → NotApplicable, aterra mas marca **não-verificado** (exit 6) | Implementado+testado | `not_applicable_lands_but_reports_unverified` |
| Validade configurável `DX_MAX_DB_AGE_SECS` | Implementado+testado | `gate_approved_when_db_fresh_lands` |
| `--offline` no `add` (resolve só do cache) | Implementado | demo offline |

### Cenários de aceitação exercitados ao vivo

- **Advisory numa dependência → bloqueio antes de build + caminho legível**:
  `smallvec@=1.6.0` / RUSTSEC-2021-0003 → `BLOQUEADO` (exit 4) com `cargo tree -i`.
- **Malware/advisory sem CVE → bloqueio via advisory**: o bloqueio acima é
  por advisory RUSTSEC, independente de CVE.
- **Crate sem advisory → apenas ausência de vulnerabilidade conhecida**: o
  relatório di-lo explicitamente (sem selo de benignidade).
- **Cache válido → reutilização sem redownload**; aviso novo (snapshot muda)
  invalida (teste unitário).
- **Base expirada → uso não autorizado (pending)**: base de 167h, grafo limpo
  → `PENDENTE` (exit 5).
- **Repetir `add` → sem mudança**; **`--dry-run` → nada escrito**;
  **conflito de origem → recusa**; **crash a meio → recuperação sem perda**.

## Totais de teste

- 23 testes no inc.1 (13 unit + 10 integração de binário).
- +9 testes de unidade no inc.2 (`dx-security`).
- **Total: 32 testes**, todos verdes; `cargo fmt --check` e `cargo clippy
  --all-targets` limpos.

## Limitações honestas (o que NÃO está feito)

- **Gate ainda não ligado a `add`/`build`**: `dx audit` opera sobre o
  `Cargo.lock` existente. A ordenação resolver-em-staging → auditar → confirmar
  dentro do `add`, **sem** executar `build.rs`/proc-macros, está desenhada
  (ADR-0005) mas não implementada.
- **Isolamento de build de dependências** (rede desligada após fetch, FS
  mínimo, limites): planeado; não chamamos "sandbox" a nada hoje.
- **Extracção segura** (path traversal, symlink escape, zip-bomb), **integridade
  de checksum** própria e **política de origem anti-confusion**: desenhadas no
  threat model; não implementadas.
- **`cargo-deny`** (origens/licenças): previsto para complementar o `cargo-audit`;
  não integrado.
- **Resolução automática de "latest"** no `add` de registry: adiada de propósito
  (tem de passar pelo gate antes de aterrar).
- **`update`/`fetch`/`release prepare`**: stubs honestos (exit 3).
- **Perfis LLVM/PGO (inc.4)** e **adaptador Seiva/`seiva-console-admin`
  (inc.5)**: não iniciados.
- **Benchmarks (contrato §9)**: não executados; o orçamento de overhead ainda
  não foi medido.
- **Nada publicado** em crates.io; o nome `dx` está ocupado (ADR-0003).

## Próximo trabalho (ordem proposta)

1. Ligar o gate ao `add` com staging isolado sem execução de código.
2. `cargo-deny` + política de origem + excepções assinadas/datadas.
3. Extracção segura + verificação de integridade própria.
4. Relatórios (JSON/JUnit/HTML) e suites de teste configuráveis (inc.3).
5. Perfis LLVM/PGO e `release prepare` (inc.4).
6. Adaptador Seiva declarativo + `seiva-console-admin` (inc.5).
7. Harness de benchmarks (10/100/1000 packages; frio/quente; fresca/expirada).
