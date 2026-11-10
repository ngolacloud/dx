# ADR-0028 — Nome de descoberta: resolução do item adiado

- Estado: Aceite (fecha o item «nome de descoberta» adiado no ADR-0025)
- Data: 2026-11-10
- Contexto: reavaliação do ADR-0003 (nome publicado vs executável)

## Reavaliação (crates.io, 2026-11-10)

| Nome | Estado | Conclusão |
|---|---|---|
| `dx` | existe, `0.0.0`, **yanked**, dono alheio | nome **reservado** — indisponível |
| `dx-cli` | existe, `0.3.1`, activo, não relacionado | indisponível |
| `dx-core` | existe, `0.3.0`, activo, não relacionado | indisponível |

A situação do [ADR-0003](0003-nome-publicado-vs-executavel.md) **mantém-se**: os
três nomes estão ocupados. O `dx` é um placeholder `0.0.0` yanked de outro dono —
um crate yanked **não liberta o nome**, pelo que não é publicável por nós.

## Decisão

Fechar o item sem mudar o nome publicado. O ponto-chave é que o **executável já
se chama `dx`** (`[[bin]] name = "dx"`), independente do nome do package. O atrito
de descoberta resume-se ao **comando de instalação**, e esse já está mitigado:

1. **Caminho primário — instalador de uma linha** (já existe, ADR-0003/release):
   `curl -fsSL https://ngolacloud.github.io/dx/install.sh | bash` → instala o
   binário `dx` (binário pré-compilado; fallback `cargo install ngoladx-cli`).
   É este o caminho a destacar na documentação.
2. **A partir do source:** `cargo install ngoladx-cli` (binário `dx`).

Depois de instalado, o utilizador escreve sempre `dx` — o nome `ngoladx-cli`
só aparece uma vez, no comando de instalação from-source.

## Não perseguido agora (registado)

- **Petição do nome `dx`** ao crates.io (política de nomes abandonados/squat): o
  `dx` está yanked em `0.0.0` e inactivo, mas o processo é manual e incerto; não
  é perseguido nesta fase. Se concedido no futuro, publicar-se-ia um shim `dx`
  que depende de `ngoladx-cli`, sem mudar o binário.
- **`cargo binstall`**: os assets de release (`dx-linux-<arch>`) não seguem o
  padrão por omissão do binstall; suportá-lo exigiria metadata
  `[package.metadata.binstall]` — fica como melhoria futura, não reivindicada.

## Consequências

- O item «nome de descoberta» fica **fechado**: decisão = manter `ngoladx-cli`
  para publicação e destacar o instalador de uma linha como via primária.
- Nenhuma mudança de código; a documentação lidera com o instalador.

## Anti-objectivos

- Não mudar o nome do executável (`dx` é estável).
- Não reivindicar `cargo binstall` até a metadata existir e ser testada.
