# Changelog

All notable changes to DX are documented here. DX follows the positioning in
[ADR-0015](docs/adr/0015-posicionamento-e-plano-de-impacto.md): explainable
changes, security before execution, predictable recovery — Cargo-compatible.

## [Unreleased]

### Changed — positioning
- **DX is the Cargo _trust layer_, not a replacement** ([ADR-0020](docs/adr/0020-camada-de-confianca-nao-substituto.md)).
  The value axis is unified trust (supply-chain/security/release), never speed:
  Cargo is already fast and unified, so DX runs _with_ it. README, `--help` and
  ADR-0015 no longer suggest replacing Cargo, and nothing promises DX is "faster
  than Cargo".

### Decided — discovery name ([ADR-0028](docs/adr/0028-discovery-name.md))
- Re-checked crates.io (2026-11-10): `dx` (0.0.0, yanked, foreign owner — name
  reserved), `dx-cli` (0.3.1, active), `dx-core` (0.3.0, active) all remain
  unavailable. Item closed with no rename: the **executable is already `dx`**;
  the one-line installer (which installs the `dx` binary) is the primary
  discovery path, with `cargo install ngoladx-cli` for from-source. Petitioning
  the yanked `dx` name and `cargo binstall` metadata are noted as future, not
  pursued.

### Added — bundle reproducibility ([ADR-0027](docs/adr/0027-bundle-reproducibility.md))
- **`dx bundle reproduce <dir>`** re-vendors from the bundle's own `Cargo.lock`
  into a scratch dir and proves the `vendor/` aggregate hash matches the one
  recorded in `bundle.json` — i.e. the dependency sources reproduce byte-for-byte
  for a given lockfile. Honest scope: the vendored sources reproduce; the
  timestamp and host environment record are excluded (not the final binary). CI
  gains a best-effort reproducibility job.

### Added — per-crate fast-BLOCK pre-screen ([ADR-0026](docs/adr/0026-per-crate-fast-block-prescreen.md))
- **Cross-project network effect, BLOCK-only.** `dx_security::prescreen` caches,
  per advisory-db snapshot, the `crate@version`s known to carry a blocking
  cargo-audit advisory (`~/.dx/advisories`, `DX_ADVISORY_DIR`). A *different*
  project pulling a known-vulnerable crate is blocked immediately, without
  running the full gate ("fast-blocked by per-crate pre-screen"). Sound by
  construction: it can only ever reach **Blocked** — never Approved; the Approved
  path always runs the full gate (audit + graph-level deny). Only per-crate-sound
  cargo-audit categories are cached (deny/graph-level excluded); the block is
  re-evaluated under the current policy and matched to the same snapshot.

### Added — Cargo compatibility contract ([ADR-0025](docs/adr/0025-cargo-compatibility-contract.md))
- **`dx compat`** probes the host Cargo against the interfaces DX depends on
  (`cargo metadata --format-version 1`, `cargo tree`, `generate-lockfile`,
  `update`, `vendor`, `rustc -vV`) and classifies the version against the tested
  range (MSRV 1.85 … 1.96); exits non-zero if a required interface is broken. A
  summary line is added to `dx doctor`. New crate `ngoladx-compat` (`dx_compat`)
  pins the contract. CI gains a toolchain matrix (`compat.yml`:
  1.85/stable/beta/nightly) that runs the suite + `dx compat`, catching Cargo
  format drift. Reproducibility and the discovery-name question are explicitly
  deferred (see ADR-0025).

### Added — global, tamper-evident decision cache ([ADR-0024](docs/adr/0024-global-decision-cache.md))
- **Shared verdict cache** at `~/.dx/verdicts/` (`DX_VERDICT_DIR`), keyed by the
  same `CacheKey`. `run_gate` now checks local → global → compute, publishing to
  both; a global hit also warms the local cache and reports "reused from GLOBAL
  cache". Worktrees/CI/teammates with the **identical** `Cargo.lock` skip
  re-running cargo-audit/cargo-deny (honest scope: reuse needs the same
  lockfile). Each entry carries an HMAC-SHA256 over `<key.digest>|<decision>`,
  signed by a machine-local key (`~/.dx/verdict.key`, 32 bytes, mode 0600);
  wrong key / tampered content / flipped decision → cache miss (fail-closed).

### Added — `dx run <script.rs>` ([ADR-0023](docs/adr/0023-run-single-file-script.md))
- **Single-file scripts with inline deps** (cargo `-Zscript`-style `---`
  frontmatter), analogous to PEP 723/`uvx`. The DX differentiator: the script's
  dependency graph passes the **same security gate** as `dx add` BEFORE it runs
  — `Blocked`/`Pending` refuse to execute. Builds an ephemeral, workspace-
  isolated package, resolves without compiling, audits, then `cargo run`. New
  crate `ngoladx-script` (`dx_script`). `dx run` still delegates to `cargo run`
  for normal projects; script mode triggers only when the first arg is an
  existing `.rs` file.

### Added — `dx toolchain` ([ADR-0022](docs/adr/0022-toolchain-pinning.md))
- **Project toolchain pinning** (`dx toolchain list|install|pin|use`), composing
  rustup. The pin is the native `rust-toolchain.toml` (works without DX); DX adds
  a provenance record (`dx-toolchain.lock`) of what the channel resolved to
  (rustc version/commit/host). `dx environment verify` now prints a
  `dx toolchain use <channel>` remediation on rustc/cargo/host drift. New crate
  `ngoladx-toolchain` (`dx_toolchain`). Hard rule: a `pin` never triggers a
  toolchain download — provenance is probed only when the exact channel is
  already installed. No fallback: absent rustup is refused honestly.

### Added — overhead-≈0 hot path ([ADR-0021](docs/adr/0021-fast-path-overhead-zero.md))
- **`exec` passthrough.** On Unix, `dx build`/`run`/`check`/`fmt`/`lint` replace
  the `dx` process image with `cargo` via `execvp` — zero fork+wait, zero
  intermediate process. Measured `dx build` ≈ `cargo build` (within noise).
- **`dx_security::fast_trust`.** A trust lookup that spawns **no** process: reads
  only `Cargo.lock` + `.dx/security/decision-*.json` to answer
  `Approved`/`Stale`/`Unknown`/`NoLockfile`. `dx build`/`run`/`check` print a
  single advisory line only when the state is not `Approved` (silent on the hot
  path); never blocks the build; silence with `DX_NO_TRUST_HINT=1`. The real
  gate stays on `dx add`/`dx audit`.

## [0.0.2] — 2026-11-05 — `ngoladx-cli`

CLI-only release (the `dx_*` libraries stay at `0.0.1`; no library code changed).

### Added
- **Dynamic shell autocomplete.** Beyond commands and flags, `dx` now completes
  the **values after parameters**, resolved at runtime from the real project
  state: crate names for `add`/`remove`/`why` (from `Cargo.toml` and the Cargo
  cache), `--template`, build `--profile`/`--profiles`, and test
  `--suite`/`--format` (including suites declared in `dx.toml`). Install with
  `source <(COMPLETE=bash dx)` (bash; also zsh/fish/elvish/PowerShell).
- **One-line installer** served from GitHub Pages:
  `curl -fsSL https://ngolacloud.github.io/dx/install.sh | bash` (wraps
  `cargo install ngoladx-cli`).

## [0.0.1] — 2026-11-05 (consolidated)

First consolidated milestone, published to crates.io under the `ngoladx-*`
prefix (the `dx`/`dx-cli`/`dx-core` names are taken by unrelated crates — see
[ADR-0003](docs/adr/0003-nome-publicado-vs-executavel.md)). The executable stays
`dx` (`[[bin]] name = "dx"`) and Rust import names stay `dx_*` (`[lib] name`).
Install with `cargo install ngoladx-cli`.

### Dependency management (Cargo-compatible)
- `dx new`/`init` (incl. `--template seiva`), `dx add` (idempotent, `--dry-run`,
  format-preserving, journaled/recoverable), `dx remove`.
- `dx add --plan`: impact before landing — new graph packages, execution
  surfaces (`build.rs`/proc-macro/native), MSRV, security verdict, cache-vs-new.
- `dx why` / `dx explain --duplicates|--feature`: explain the graph (Cargo tree).
- `dx update --plan|--security|--apply`: minimal, validated, recoverable updates.

### Security before execution
- Gate wired into `dx add` (resolve candidate in isolated staging without
  `build.rs`, then `cargo-audit` + `cargo-deny`, then land only if approved).
- Decision cache; configurable advisory-base validity; honest pending/blocked.
- `dx audit`, `dx doctor`, `dx fetch` (integrity + safe extraction),
  `dx build --isolated` (real network/env/resource isolation).

### Build, test, release
- LLVM profiles (`dx build --profile`), measurement (`dx bench-build`), PGO
  (`dx pgo`), dependency-pipeline benchmark (`dx bench`).
- Configurable test suites + versioned reports (`dx test` → terminal/JSON/JUnit/
  HTML), with secret-scrubbing and HTML escaping.
- `dx release prepare` (validations + SBOM + provenance + topological publish
  order + checksums; prepare ≠ publish).

### Environment, tools, bundles
- `dx environment capture/verify` (reproducible env record + drift).
- `dx tool install/run/list` (project-pinned tools, provenance, anti-tamper,
  `run --isolated`).
- `dx bundle export/verify` (offline verifiable bundle: env + SBOM + vendor).

### Seiva (optional)
- `dx new --template seiva`, `dx add seiva-console-admin` (declarative,
  anchor-based integration; never runs crate hooks).

### Notes
- All DX output, logs, `--help` and errors are in English.
- 111 tests; `cargo fmt --check` and `cargo clippy --all-targets` clean.
- Honest limits are documented per ADR and in `docs/progress.md`.
