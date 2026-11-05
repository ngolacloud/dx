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
`dx lint`. Delegam no Cargo com argv estruturado (sem shell), herdando stdio e
código de saída. **Ainda não aplicam o gate de segurança** (o `add` aplica).

### `dx test --suite <nome> [--format terminal|json|junit|html] [--out <ficheiro>] [--no-ansi]`
Corre uma suite e emite relatório. Suites: `unit`/`doc`/`integration` (builtins
Cargo) ou definidas em `dx.toml` (`[test.suites.<nome>]` com `runner`+`kind`
rust|command|load). Nome sem config é recusado (§8). JSON é versionado; HTML
auto-contido; segredos removidos e HTML/XML escapados. Erro de runner nunca fica
verde. Suites `load` falham por thresholds (p95/p99/erros/rps) lidos de
`DX_LOAD_METRICS` / `target/dx-load-metrics.json`.

### `dx build --profile <dev|release|throughput|size> [--native] [--measure]`
Perfis LLVM aplicados por `cargo --config` (sem tocar no Cargo.toml). `release`
= ThinLTO+cg16 (default equilibrado); `throughput` (fat-LTO+cg1) é opt-in e
lento; `size` faz strip. `--native` adiciona target-cpu=native (com aviso; não
para releases distribuídas). `--measure` reporta tempo/memória/tamanho.

### `dx bench-build [--profiles a,b,c]`
Build **limpo** por perfil, tabela de tempo (relógio), memória máxima (GNU time)
e tamanho do binário.

### `dx pgo [--bin <nome>] [-- <workload...>]`
PGO real: instrumenta (`-Cprofile-generate`), corre o workload (o binário ou um
comando dado), funde com `llvm-profdata` (prefere o da sysroot, versão
compatível; reporta incompatível), recompila (`-Cprofile-use`) e compara. O
ganho de performance exige medir o serviço com carga (ver §9).

### `dx build --isolated`
Compila com isolamento **real** (crate `dx-sandbox`): rede desligada via
`unshare(CLONE_NEWUSER|CLONE_NEWNET)`, ambiente sem segredos (allowlist mínima),
limites de recursos (`setrlimit`). Hermético: usa `--offline` e desliga
`rustc-wrapper` de rede. Reporta exactamente o que foi imposto. Isolamento de
**filesystem não está imposto** nesta versão (reportado `false`).

### `dx fetch [--locked] [--extract-check]`
Verifica a integridade do grafo bloqueado **sem compilar**: checksum dos
`.crate` em cache vs `Cargo.lock`. `--extract-check` desempacota cada um com
salvaguardas (traversal/symlink/zip-bomb) para um temporário e descarta.
Mismatch ou extracção insegura → BLOQUEADO (exit 4). `--locked` exige Cargo.lock.
Packages não descarregados ficam "sem cobertura".

### `dx release prepare`
Valida em ambiente limpo (fmt/clippy-D/test/build + gate de segurança; API via
cargo-semver-checks se presente) e produz artefactos para revisão em
`target/dx-release/`: SBOM CycloneDX, proveniência, ordem topológica de
publicação e binários+checksums. **Prepara ≠ publica**: não cria tags, não muda
versões, não publica; é idempotente. Base expirada ⇒ NÃO PRONTO (§5).

### `dx new <nome> --template seiva`
Scaffola uma plataforma Seiva com composição gerida pelo DX (`src/composition.rs`
com âncoras `// dx:seiva:*` + `[package.metadata.dx.seiva].composition`).

### `dx add seiva-console-admin`
Adiciona a dependência (via gate) e, se a composição for gerida, integra a
extensão por âncoras (idempotente, reversível) — nunca correndo hooks do crate.
Composição não gerida → dependência adicionada + **patch pendente** (estado
parcial, exit 6), sem alegar integração. Sem permissões admin; recompila com
`dx build`. (A extensão de registry exige versão; fictícia demonstrada via
`--path`.)

## Planeado

`dx update`. Hoje devolve exit 3 com mensagem a dizer em que incremento chega.
