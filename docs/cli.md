# CLI do `dx`

Convenções: stdout é para saída estruturada; o plano e o progresso vão para
stderr. Códigos de saída: `0` sucesso · `1` erro · `3` comando ainda não
implementado (planeado) · código do Cargo em passthrough.

## Implementado (inc.1)

### `dx new <nome> [--lib|--bin]`
Cria um projecto novo. Recusa directório não vazio. Imprime o plano; `--dry-run`
não escreve.

### `dx init [--lib|--bin]`
Como `new`, no directório actual. Recusa se já houver `Cargo.toml`.

### `dx add <spec> [opções]`
Adiciona/actualiza uma dependência de forma idempotente.
- `spec`: `nome` ou `nome@<req>`.
- Origem: `--git <url>` com `--rev`/`--tag`/`--branch`; `--path <dir>`;
  `--registry <nome>`.
- `--features a,b` · `--no-default-features` · `--optional`.
- Secção: `--dev` · `--build`.
- `--manifest-path <Cargo.toml>` · `--dry-run`.

Regras: features normalizadas (ordem/dups irrelevantes); repetir é no-op;
mudar de origem é erro (remover primeiro); registry **sem** versão é recusado
(usar `nome@<req>`); nome suspeito (typosquat) é recusado com sugestão.

**Gate de segurança no `add`** (ADR-0006): antes de aterrar, o DX resolve o
grafo candidato num staging isolado (`cargo generate-lockfile`, **sem** correr
`build.rs`) e corre o gate. Só aterra se **Approved**; **Blocked** (exit 4) e
**Pending**/base expirada (exit 5) não aterram. Projectos workspace/path →
**NotApplicable**: aterra mas marca não-verificado (exit 6, correr `dx audit`).
`--offline` resolve só do cache; validade da base por `DX_MAX_DB_AGE_SECS`.

### `dx audit` / `dx doctor`
`dx audit` corre o gate sobre o `Cargo.lock` do projecto e reporta achados por
categoria, estado da base e decisão (exit 0/4/5). `dx doctor` diagnostica
toolchain, `cargo-audit`, frescura da advisory-db e política.

### `dx remove <nome> [--dev|--build] [--manifest-path] [--dry-run]`
Remove uma dependência; idempotente (remover o que não existe é no-op).

### Passthrough estruturado para Cargo
`dx check` · `dx build [--release]` · `dx run [-- args]` · `dx fmt [--check]` ·
`dx lint` · `dx test [--suite unit|doc|integration]`.
Delegam no Cargo com argv estruturado (sem shell), herdando stdio e código de
saída. **Ainda não aplicam o gate de segurança** (inc.2 liga-o antes do build).

## Planeado

`dx fetch --locked` · `dx update` · `dx release prepare` (inc.4) ·
`dx new <plat> --template seiva` e `dx add seiva-console-admin` (inc.5).
Hoje devolvem exit 3 com mensagem a dizer em que incremento chegam.
