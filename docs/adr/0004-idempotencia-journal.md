# ADR-0004 — Idempotência, journal e recuperação de crash

- Estado: Aceite
- Data: 2026-11-05
- Contexto do contrato: §4

## Decisão

Toda a operação local mutadora:

1. produz um **plano** antes de tocar em ficheiros (`--dry-run` imprime-o sem
   aplicar);
2. corre sob um **lock de workspace** exclusivo e consultivo em `.dx/dx.lock`
   (`fs4`), serializando operações concorrentes;
3. aplica escritas de forma **transaccional** via journal.

### Protocolo de journal (`dx-manifest::txn`)

Como renomear vários ficheiros não é uma transacção atómica única:

1. escrever `.dx/journal.json` com o estado final pretendido (caminho +
   sha256 + conteúdo) e `fsync`;
2. aplicar cada escrita por `tmp + fsync + rename` (promoção atómica; um
   ficheiro parcial nunca é promovido);
3. apagar o journal (commit) e `fsync` do directório.

### Recuperação

No arranque de qualquer comando mutador chama-se `recover`: se existir um
journal, cada alvo cujo hash não bate é reescrito (roll-forward idempotente) e o
journal é consumido. Um journal corrompido é descartado sem tocar nos alvos
(as escritas de alvo são atómicas, logo o original permanece íntegro).

## Prova

- Unit: `transaction_applies_and_recovers`, `repeat_add_is_noop`,
  `preserves_comments_and_other_entries`, `source_conflict_is_error...`.
- Integração (binário real): `add_is_idempotent_byte_for_byte`,
  `dry_run_writes_nothing`, `pending_journal_is_recovered_on_next_op`,
  `remove_is_idempotent`.
- Ao vivo: dois `dx add serde@1 --features derive` seguidos → sha256 do
  Cargo.toml idêntico.

## Limites honestos

- O lock é **consultivo**: protege contra dois processos DX, não contra quem
  edite Cargo.toml por fora ignorando o lock, nem contra um atacante com
  controlo total do mesmo utilizador (ver threat model).
- A detecção de alteração concorrente por humano entre o plano e a aplicação é
  parcial no inc.1 (comparação no momento da leitura); endurecimento
  (fingerprint + recusa) está previsto com o gate do inc.2.
