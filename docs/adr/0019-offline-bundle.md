# ADR-0019 — `dx bundle`: offline, verifiable build bundle

- Estado: Aceite (implementado)
- Data: 2026-11-05
- Contexto: directriz de produto do dono (ambiente reproduzível / cenários
  on-premises N'GolaCloud com conectividade limitada)

## Decisão

`dx bundle export` produz um directório **auto-contido e verificável**, para
consumir offline:

- `Cargo.toml` + `Cargo.lock` (grafo fixado);
- `environment.json` (registo de ambiente — ADR-0017);
- `sbom.cdx.json` (CycloneDX — ADR-0011);
- `vendor/` + `cargo-config.toml` (fontes vendorizadas via `cargo vendor`, com o
  snippet `[source.*]` para build offline);
- `bundle.json`: manifesto com **sha256 por ficheiro de topo** e um **hash
  agregado** de `vendor/` (determinístico: sha256 das linhas ordenadas
  `relpath:filehash`).

`dx bundle verify <dir>` recomputa os hashes (ficheiros + agregado de `vendor/`)
e falha (exit 4) em qualquer divergência — detecta adição, remoção ou alteração.

## Honestidade

- Pacote **verificável** offline, **NÃO** uma afirmação de reprodutibilidade
  byte-a-byte. `cargo vendor` precisa de rede na exportação (consome-se offline
  depois).
- O agregado de `vendor/` é integridade do conteúdo vendorizado contra o próprio
  bundle, não prova de benignidade das fontes.

## Prova

- Integração (CLI): verify sem `bundle.json` falha; export sem lockfile falha.
- Ao vivo: export junta os 5 ficheiros + `vendor/` (12 ficheiros, agregado);
  verify bate (exit 0); alterar um ficheiro vendorizado → MISMATCH no agregado
  (exit 4).
