# DX — release consolidation

Consolidated, release-ready state of DX. **Nothing is published**; this records
the gates that pass and the artifacts `dx release prepare` produces for review.
Publishing requires an explicit instruction (ADR-0011).

## Identity

- Executable: `dx`. Published crate: `dx-cli` (the `dx` name is taken on
  crates.io — [ADR-0003](docs/adr/0003-nome-publicado-vs-executavel.md)).
- Version line: `0.0.1` (workspace). Dual-licensed **MIT OR Apache-2.0**
  (`LICENSE-MIT`, `LICENSE-APACHE`).
- Toolchain: Rust 1.96.0 / LLVM 22; MSRV declared `1.82`.

## Self-validation (dogfood: `dx release prepare` on DX itself)

All gates pass with a **fresh advisory database** (refreshed per ADR-0005):

| Gate | Result |
|------|--------|
| fmt --check | OK |
| lint (clippy -D warnings) | OK |
| tests (111) | OK |
| clean build (release) | OK |
| security (DX gate: cargo-audit + cargo-deny) | OK |
| semver (API) | NOT COVERED — install `cargo-semver-checks` |

Artifacts in `target/dx-release/`:
- `sbom.cdx.json` — CycloneDX SBOM, 109 components.
- `provenance.json` — rustc 1.96.0, host x86_64-unknown-linux-gnu, git commit,
  binary `dx` (~3.79 MiB) + sha256.
- `publish-order.txt` — topological order (13 crates, `dx-cli` last).

## CI

`.github/workflows/ci.yml`: fmt + clippy(-D) + test + release build on every
push/PR; an optional `security` job runs `dx audit` after refreshing the base.

## When publishing is authorized (NOT done here)

1. Decide/confirm the published name (`dx-cli` or another; `dx` is taken).
2. `dx release prepare` with a fresh advisory base → review `target/dx-release/`.
3. Publish crates in `publish-order.txt` order (deps first), via `cargo publish`
   per crate, with an explicit go-ahead.

## Honest remaining work (non-blocking)

Prebuilt tool binaries (cargo-binstall under policy), filesystem isolation
(mount namespace), DX's own compile cache, and real download/network paths in
`dx bench` (§9). See `docs/progress.md` and the ADRs for scope and limits.
