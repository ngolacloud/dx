# ADR-0008 — Integridade de artefactos e extracção segura

- Estado: Aceite (implementado; crate `dx-integrity`)
- Data: 2026-11-05
- Contexto do contrato: §5.4 (integridade), "Defesa contra crates maliciosas"
  (extracção segura), cenário de aceitação "Checksum inválido / archive
  traversal → Rejeição e nenhum escape do directório"

## Decisão

Crate dedicado `dx-integrity` (manuseio de arquivos + segurança de filesystem é
uma preocupação distinta do scanner/política). Dois papéis:

### 1. Integridade vs origem (`verify.rs` + `lockfile.rs`)

O Cargo.lock fixa, por package de registry, o `checksum` sha256 do `.crate`. O
DX verifica os `.crate` **em cache** contra esse checksum. Checksum prova
integridade **em relação à origem fixada**, não benignidade. Packages sem
`.crate` em cache ficam **"sem correspondência conhecida"**, nunca "íntegros".
Git/path (sem checksum de registry) ficam fora deste caminho.

### 2. Extracção segura (`extract.rs`)

`safe_unpack` desempacota um `.crate` (tar.gz) bloqueando, **antes** de escrever
cada entrada:

- **path traversal**: recusa caminhos absolutos e qualquer componente `..`
  (só componentes `Normal` são aceites — escape torna-se impossível);
- **symlink/hardlink**: recusados por completo (conservador; `.crate` normais
  não os usam);
- **zip-bomb**: limites de `max_entries`, `max_total_bytes` e `max_file_bytes`;
  a cópia limita os **bytes reais** lidos (não confia no tamanho do header);
- tipos de entrada não-ficheiro/dir (fifo, device…) recusados.

## CLI

`dx fetch [--locked] [--extract-check]`: verifica o checksum dos `.crate` em
cache contra o Cargo.lock e, com `--extract-check`, desempacota cada um para um
temporário com as salvaguardas e descarta. Mismatch ou extracção insegura →
**BLOQUEADO** (exit 4). `--locked` exige Cargo.lock.

## Prova

- Unit (`dx-integrity`, 9 testes): traversal, symlink, zip-bomb (total e
  por-ficheiro), demasiadas entradas, absoluto, adulteração detectada, parse de
  lockfile.
- Ao vivo: `dx fetch --extract-check` no projecto DX → 96 crates verificados
  (checksum + extracção segura). `.crate` adulterado em `CARGO_HOME` isolado →
  BLOQUEADO (exit 4) com o sha256 esperado vs obtido.

## Limites honestos

- Só verifica o que **está em cache**; o resto é "sem cobertura".
- Não protege contra um atacante com controlo total do mesmo utilizador/sistema
  (TOCTOU entre verificar e o Cargo consumir fica fora de âmbito).
- A extracção segura é a fundação para uma futura quarentena própria de unpack;
  hoje o build real continua a usar o cache do Cargo.
