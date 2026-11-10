# ADR-0020 — DX é a camada de confiança do Cargo, não um substituto

- Estado: Aceite (directriz do dono; reposicionamento face à avaliação UV vs DX)
- Data: 2026-11-10
- Contexto: avaliação honesta UV vs DX e plano de melhorias

## Problema

O enquadramento informal «fazer as pessoas deixarem o Cargo e usarem o DX»
copia a estratégia do [UV](https://docs.astral.sh/uv/) — mas copia-a para um
terreno onde ela não se aplica.

O UV destronou o `pip` porque o ecossistema Python era **lento e fragmentado**:
`pip` + `venv` + `pyenv` + `pip-tools` + `poetry` + `pipx`, cada um com o seu
formato, em série. O UV reescreveu tudo em Rust (resolver PubGrub, index client,
instalador com copy-on-write, gestor de interpretador) e colapsou-os num binário
10–100× mais rápido. A dor era real e enorme.

**O Cargo não é o pip.** O Cargo já é aquilo que o UV aspirava tornar o pip: um
único binário rápido, com lockfile determinista, resolver sólido, workspaces e
cache global. Logo:

- O DX **não pode** vencer o Cargo em velocidade de resolução/compilação.
- O DX **não pode** vencer por «colapsar 5 ferramentas em 1» — o Cargo já é 1.

Copiar a estratégia do UV (substituir tudo, vencer por velocidade) seria
construir as features erradas.

## Decisão

O DX **não** substitui o Cargo. O DX é a **camada de confiança** que corre
*com* o Cargo:

> **DX — a camada de confiança do Cargo: supply-chain, segurança e release
> num só comando, com a garantia a acontecer _antes_ da execução.**

O eixo de valor do DX **não** é velocidade (eixo do UV); é **confiança
unificada**. A stack que hoje está fragmentada no mundo Rust não é a de
ambiente/velocidade — é a de **cadeia-de-abastecimento**:

> `cargo-audit` + `cargo-deny` + `cargo-vet` + `cargo-auditable` +
> `cargo-cyclonedx` + `cargo-dist` + scripts de vendoring.

O DX colapsa *essa* stack num só binário, com um modelo mental único, e —
diferenciador — **fá-lo antes do build, não depois** (`dx add --plan`, gate no
`add`, journal recuperável, SBOM/proveniência no `release prepare`).

## Consequências

1. **Compor, nunca substituir.** `cargo metadata`/`tree`/`build`/`rustc`
   continuam a resolver e compilar (ADR-0002). Confirma-se a filosofia de
   todas as ADRs anteriores.
2. **Invisível no caminho quente (regra dura).** Como o DX não ganha em
   velocidade, no mínimo não pode custar. O passthrough (`dx build`/`run`/
   `test`/`check`) tem de ter **overhead ≈ 0** vs Cargo — ver ADR-0021
   (fast-path + `exec`). Qualquer taxa mensurável é um bug de adopção.
3. **A alavanca de adopção é a confiança, não a simplicidade de velocidade.**
   Um comando em vez de oito ferramentas de supply-chain, com a garantia a
   acontecer antes da execução.
4. **Linguagem corrigida.** README, `--help` e ADR-0015 deixam de sugerir
   «substituir o Cargo»; passam a «corre com o Cargo». Nenhum material do DX
   promete ganhos de velocidade sobre o Cargo.
5. **Roadmap re-priorizado** (plano de melhorias):
   - P0 fast-path overhead-≈0 no passthrough (ADR-0021).
   - P1 `dx toolchain` (análogo `uv python`), `dx run <script.rs>` com deps
     inline (análogo PEP 723/`uvx`), cache de decisões global/partilhável.
   - P2 matriz de compatibilidade Cargo, reprodutibilidade, descoberta do nome.

## Anti-objectivos

- Não reimplementar o resolver nem o compilador do Cargo.
- Não prometer, em lado nenhum, que o DX é «mais rápido que o Cargo».
- Não introduzir um formato de lockfile/manifesto próprio: o `Cargo.toml`
  gerado é Cargo puro (critério «Cargo sem DX»).
