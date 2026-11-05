# ADR-0018 — `dx tool`: project-pinned developer tools

- Estado: Aceite (implementado; crate `dx-tool`)
- Data: 2026-11-05
- Contexto: directriz de produto do dono (P1 — instalação de ferramentas com
  proveniência e versões isoladas)

## Decisão

Ferramentas **não são** bibliotecas: `dx add` gere `[dependencies]`; `dx tool`
gere **executáveis fixados por projecto**.

- `dx tool install <name>[@version] [--path <dir>]` — fixa a ferramenta em
  `dx-tools.toml`, instala-a num directório **local ao projecto** (`.dx/tools`)
  via `cargo install --root .dx/tools --locked`, e regista **proveniência**
  (origem + versão + sha256 do binário) em `.dx/tools/dx-tools.lock`. Compilar a
  versão fixada é o caminho verificável por omissão; `--path` instala um crate
  local (controlado/offline). **Sem fallback silencioso**: se a instalação
  falhar, falha — não cai para um binário de sistema.
- `dx tool run <name> [--isolated] [-- args]` — corre **apenas** a ferramenta
  fixada no projecto, após verificar o sha256 contra o lock (**anti-tamper**).
  Nunca cai para um binário do sistema. `--isolated` corre sob o sandbox real
  (rede desligada, ambiente sem segredos — ADR-0009).
- `dx tool list` — ferramentas fixadas, versão e estado/proveniência.

## Honestidade

- **Binários pré-compilados** (via cargo-binstall quando a política aceitar a
  origem/proveniência) são um caminho **futuro**; hoje o default compila a
  versão fixada (verificável). Não se estende esta promessa a bibliotecas Rust
  pré-compiladas arbitrárias (dependem de toolchain/target/features).
- Para `--path`, o nome do bin do crate tem de coincidir com `<name>`.
- Isolamento no `run` é opt-in (`--isolated`); a instalação compila código e
  **não** corre sob sandbox (precisa de rede/build) — documentado, não fingido.

## Prova

- Unit (`dx-tool`, 4 testes): parse de `dx-tools.toml` (formas curta/completa/
  path), upsert idempotente, roundtrip+find do lock, render determinístico.
- Integração (CLI): `run` sem instalar recusa (sem fallback); `list` vazio.
- Ao vivo: `install --path` → proveniência (source+sha256) + pin em
  dx-tools.toml; `run -- hello world` verifica e corre; **tamper** no binário →
  sha256 não bate → recusa; `run` antes de instalar → recusa.
