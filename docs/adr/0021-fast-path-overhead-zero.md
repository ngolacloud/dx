# ADR-0021 — Fast-path de confiança e passthrough overhead-≈0

- Estado: Aceite (implementado em `feat/trust-layer-overhead-zero`)
- Data: 2026-11-10
- Contexto: ADR-0020 (camada de confiança) exige que o passthrough não custe

## Problema

O ADR-0020 fixa a regra dura: como o DX não ganha ao Cargo em velocidade, **não
pode custar**. Dois custos reais existiam no passthrough (`dx build`/`run`/
`check`/`fmt`/`lint`):

1. **Processo a mais.** `cargo_passthrough` fazia `Command::spawn(cargo)` e
   esperava — ou seja, a árvore de processos tinha `dx` → `cargo`, com `dx` a
   bloquear à espera do filho. Overhead de um processo inteiro por invocação.
2. **Armadilha do gate.** Calcular a `CacheKey` completa do veredicto (para
   saber se o estado já foi aprovado) implica `cargo audit --version` e
   `cargo deny` — spawns de dezenas de ms. Qualquer «verificação barata» que
   construísse a chave completa no caminho quente anulava o objectivo.

## Decisão

### 1. `exec` em vez de `spawn` (Unix) — zero processo extra

No passthrough puro, substituir a imagem do processo `dx` pelo `cargo` via
`execvp` (`std::os::unix::process::CommandExt::exec`). O `dx` **deixa de
existir**; o `cargo` herda o PID e o seu código de saída torna-se o do comando.
Resultado: zero fork+wait, zero processo intermédio — o custo passa a ser o do
parse do clap (sub-milissegundo), indistinguível do ruído de medição.

Em plataformas não-Unix mantém-se `spawn`+`status` (fallback correcto).

### 2. `fast_trust` — consulta de confiança sem spawn

Uma função `dx_security::fast_trust(root, manifest_dir) -> TrustState` que
responde «este lockfile já foi aprovado?» lendo **apenas** ficheiros:

- `sha256(Cargo.lock)` (barato);
- os veredictos em `.dx/security/decision-*.json` (JSON pequeno).

**Nunca** corre `cargo-audit`/`cargo-deny`/`rustc`. Casa pelo `lockfile_sha`
(a parte que muda quando as dependências mudam), devolvendo:

| Estado | Significado | Caminho quente |
|---|---|---|
| `Approved` | existe veredicto `approved` para este exacto lockfile | **silêncio** |
| `Stale` | há veredictos, mas o lockfile mudou (ou o último não foi aprovado) | nota 1 linha |
| `Unknown` | o projecto nunca estabeleceu baseline de confiança | nota 1 linha |
| `NoLockfile` | nada resolvido ainda | silêncio |

Honestidade: o `fast_trust` ignora de propósito drift de política/base de
advisories — é um **sinal barato**, não o gate. Se a política ou a base mudaram,
o `dx audit`/`dx add` (que correm o scanner real) reavaliam. A nota nunca
bloqueia o build; só aponta para `dx audit`. Silenciável com
`DX_NO_TRUST_HINT=1`.

## Consequências

- `dx build`/`run`/`check`/`fmt`/`lint` têm overhead ≈ 0 vs `cargo` (medível com
  `hyperfine 'cargo build' 'dx build'` em projecto sem alterações).
- A camada de confiança fica **presente mas invisível**: em estado estável
  (depois de um `dx audit`) o passthrough é silencioso; só fala quando o estado
  deixou de ser de confiança.
- O gate real continua a correr onde deve (ADR-0005/0006): no `dx add` e no
  `dx audit`. O `fast_trust` não o substitui — apenas evita pagá-lo no caminho
  quente quando nada mudou.

## Anti-objectivos

- Não transformar `dx build` num gate bloqueante (quebraria adopção e a
  fronteira «o gate é no `add`»).
- Não spawnar nenhum processo no `fast_trust`.
- Não aprovar nada a partir do `fast_trust`: aprovar é sempre do gate real.
