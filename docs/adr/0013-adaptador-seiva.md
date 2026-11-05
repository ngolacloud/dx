# ADR-0013 — Adaptador Seiva (integração declarativa)

- Estado: Aceite (implementado; crate `dx-seiva`)
- Data: 2026-11-05
- Contexto do contrato: §6

## Decisão

`dx add <extensão-seiva-conhecida>` adiciona a dependência (passando pelo gate)
e, se o projecto tiver uma **composição gerida**, liga a extensão via um
**protocolo declarativo versionado** interpretado por código **confiável do DX**
(`dx-seiva`). NUNCA corre hooks/scripts fornecidos pela extensão.

### Protocolo (DX-side, não vem do crate)

`ExtensionSpec { protocol_version, crate_name, feature, use_line, register_line,
summary }`. Registo confiável em `dx_seiva::known(name)` — hoje só
`seiva-console-admin` (nome canónico; typosquat é recusado com sugestão).

### Composição gerida por âncoras

Um ficheiro declarado em `Cargo.toml` via
`[package.metadata.dx.seiva] composition = "src/composition.rs"` com regiões
`// dx:seiva:<região>:begin`/`end`. O adaptador faz **upsert idempotente** da
`register_line` dentro da âncora (uma só ocorrência) e preserva indentação. É
**reversível** (`revert` remove a linha gerida, mantendo as âncoras). A escrita
usa a mesma transacção (journal+lock) do resto do DX.

### Caso não gerido → estado parcial (não fingir)

Sem a metadata/âncoras, a dependência **aterra** mas a integração **NÃO** é feita:
imprime o patch pendente (âncoras + `register_line`) e sai com **6** (parcial).
Nunca alega integração concluída (contrato §6).

### Fronteiras

- A geração de módulos de negócio/contratos/migrações continua a pertencer à
  CLI/biblioteca Seiva; o DX só faz a cablagem (dep + âncora + registo).
- Uma dependência no Cargo.toml não se liga magicamente: é preciso **recompilar**
  (`dx build`). Em workspace, `--manifest-path`/`--package` selecciona o alvo.
- Não concede permissões administrativas nem cria passwords.
- `dx new --template seiva` scaffola uma plataforma com composição gerida.

## Prova

- Unit (`dx-seiva`, 4 testes): integração idempotente (1 ocorrência), reversão,
  âncoras em falta reportadas (não fingidas), indentação preservada.
- Ao vivo: `dx new --template seiva` → `dx add seiva-console-admin --path <ext>`
  → âncora preenchida, `dx run` imprime "[console-admin] registado"; repetir não
  duplica; projecto sem composição gerida → ESTADO PARCIAL (exit 6) com a
  dependência à mesma adicionada.

## Limites honestos

- `dx add` de registry exige versão (`seiva-console-admin@<req>`); a extensão
  fictícia foi demonstrada via `--path`.
- A assinatura exacta de `register()` é codificada por-extensão no adaptador
  (reflecte o contrato da extensão Seiva real); aqui é a forma mínima.
- Re-integração quando a dependência já está presente mas a composição foi
  editada à mão não re-dispara (o `add` é no-op sobre a dep); usar reversão +
  novo add se necessário.
