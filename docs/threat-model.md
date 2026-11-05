# Threat model do DX

- Data: 2026-11-05
- Âmbito: incremento 1 implementado + superfícies previstas (inc.2+).

## Activos a proteger

- Integridade de `Cargo.toml`/`Cargo.lock` e configuração gerida.
- A decisão de **o que entra no grafo** e **o que é autorizado a compilar/correr**.
- A máquina do programador / runner de CI (execução de código de dependências).

## Agentes e capacidades assumidas

| Agente | Capacidade | No âmbito? |
|--------|-----------|------------|
| Crate malicioso em registry | `build.rs`/proc-macro/código nativo executável; nome typosquat; mudança de origem | **Sim** |
| Rede/registo adversário | servir artefacto adulterado; dependency confusion | **Sim** |
| Processo DX concorrente / crash | corromper o manifesto a meio da escrita | **Sim** |
| Atacante com controlo total do mesmo utilizador/SO | ler/escrever tudo, correr como o utilizador | **Não** (fora de âmbito; documentado) |

## Superfícies e mitigações

### S1 — Execução de código de dependências (inc.2)
`build.rs`, proc-macros e código nativo são **superfícies de execução**. O gate
resolve o grafo candidato **sem** os executar; inspecciona-os como dados. Para
builds de dependências, prevê-se isolamento por plataforma (sem segredos, rede
desactivada após fetch, FS mínimo, limites de recursos) — **mostrando
capacidades reais**, sem chamar "sandbox" a um mero subprocesso. Fixtures
maliciosas de teste são **inertes**.

### S2 — Dependency confusion / mudança de origem (inc.2)
Política de origem: **nunca** fazer fallback de package privado para crates.io.
Mudança de origem de uma dependência existente é erro no `add` (ADR-0002),
exige acto explícito. Registry privado indisponível → falha, **sem** fallback
público.

### S3 — Typosquatting (inc.1, sinal)
`dx add` mede distância de edição a nomes conhecidos (incl.
`seiva-console-admin`) e **recusa com sugestão** em vez de instalar o parecido.
É um sinal, não prova; popularidade/idade/reputação não são prova de segurança.

### S4 — Extracção de arquivos (**implementado**, ADR-0008)
`dx-integrity::safe_unpack` bloqueia path traversal (absoluto/`..`),
symlinks/hardlinks e descompressão descontrolada (limites de entradas/tamanho
total/por-ficheiro, limitando os **bytes reais** lidos). Provado por testes com
tarballs hostis forjados e ao vivo (`dx fetch --extract-check`).

### S5 — Integridade (**implementado**, ADR-0008)
`dx-integrity` verifica o sha256 dos `.crate` em cache contra o `checksum`
fixado no Cargo.lock (`dx fetch`). Git fixado por commit+origem (modelo);
fingerprint de conteúdo para path ainda planeado. Checksum prova integridade vs
origem, **não** benignidade. O que não está em cache fica "sem cobertura".

### S6 — Corrupção por concorrência/crash (inc.1, **implementado**)
Lock de workspace consultivo + journal com roll-forward idempotente + escrita
atómica `tmp+fsync+rename`. Um ficheiro parcial nunca é promovido; um journal
corrompido é descartado sem tocar nos alvos. Provado por teste.

### S7 — Cache TOCTOU / troca de conteúdo (inc.2)
Validar o conteúdo **efectivamente consumido** pelo build, não o que foi
descarregado. Não alegamos protecção contra atacante com controlo total do
mesmo utilizador (fora de âmbito).

### S8 — Enfraquecimento silencioso de política (inc.2)
`dx.toml` de repositório desconhecido não pode enfraquecer a política da
organização. Sem `--skip-security` genérico, sem ignore global; excepções
assinadas, datadas e com expiração, validadas na CI de release.

## Fora de âmbito (declarado)

- Atacante root/local com controlo total da conta do utilizador.
- Garantia de detecção imediata de malware zero-day: a validade da base é uma
  janela, não uma prova de detecção.
- Invocar `cargo` directamente contorna o gate DX — por desenho (DX é opcional).
