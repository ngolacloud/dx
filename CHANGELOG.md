# Changelog

All notable changes to DX are documented here. DX follows the positioning in
[ADR-0015](docs/adr/0015-posicionamento-e-plano-de-impacto.md): explainable
changes, security before execution, predictable recovery — Cargo-compatible.

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
