# ADR-0005 — Gate de segurança: download e autorização são etapas distintas

- Estado: **Proposto** (desenho do incremento 2; parcialmente implementado)
- Data: 2026-11-05
- Contexto do contrato: §5 inteiro

## Princípio

**Descarregar bytes ≠ autorizar uso.** O download pode terminar; nada
não-aprovado segue para build/run/install através do DX. A autorização espera
por uma **decisão válida**, não por N pedidos remotos em série.

## Pipeline (desenho)

1. **Resolver** o grafo candidato num staging isolado usando Cargo, **sem**
   executar `build.rs`, proc-macros, testes, exemplos ou hooks. Inspeccionar a
   configuração de ferramentas do projecto antes de invocar comandos que possam
   executar código.
2. **Consultar advisories locais** (RustSec advisory-db) para todas as
   dependências directas e transitivas. Actualização incremental da base em
   paralelo com downloads; deduplicar actualização concorrente.
3. **Downloads concorrentes** com limites de rede/memória/CPU, reutilizando o
   transporte/cache do Cargo no MVP.
4. **Integridade**: checksum dos arquivos de registry contra metadados
   confiáveis; git fixado por commit+origem; path por fingerprint de conteúdo
   invalidada quando os ficheiros mudam. Checksum prova integridade **em relação
   à origem**, não benignidade.
5. **Decisão** sobre o grafo completo **antes** de confirmar a adição e **antes**
   de qualquer compilação/execução. Bytes já no cache do Cargo são tratados como
   conteúdo não-executável até este gate. Cache existente nunca dispensa
   revalidação.
6. **Falha**: manter o manifesto original, mostrar a dependência bloqueadora e o
   caminho de dependências legível, com solução concreta.

## Categorias distintas (nunca confundir)

`vulnerability` (RUSTSEC/CVE) · `malware` (bloqueio duro) · `yanked` ·
`unmaintained` · `integrity-failure` (bloqueio duro) · `source-not-allowed`
(bloqueio duro). Política explícita por categoria; sem `--skip-security`
genérico e sem ignore global.

## Validade da base e cache de decisão

- Validade máxima configurável; baseline: **24 h** em dev, **refresh obrigatório
  antes de release**.
- Base válida → avaliar sempre localmente ou reutilizar veredicto com chave
  válida. Base **expirada** → não autorizar novas adições/builds/execução/release
  até renovar. Download isolado pode terminar com estado **pending**, nunca
  **approved**.
- Offline só aprova se base + artefactos + política forem válidos; se expirou,
  falha com instrução de actualização — nunca converter indisponibilidade em
  sucesso.
- Chave do cache de decisão: grafo + checksums/revisões + targets/features +
  versão do scanner + hash da política + snapshot da base + expiração. Aviso
  novo invalida veredictos antigos mesmo com downloads intactos.

## Excepções

Identificam advisory/package/version, motivo técnico, responsável, evidência e
expiração. A CI de release valida também a validade das excepções. Política da
organização não pode ser enfraquecida por um `dx.toml` de repositório
desconhecido.

## Honestidade de cobertura

Regista exactamente o que foi verificado e o que não tem cobertura. Git/path
privados sem advisory correspondente ficam **"sem correspondência conhecida"**,
nunca "comprovadamente seguros". Ausência de CVE **não** prova ausência de
código malicioso.

## Estado de implementação

O inc.1 **não** liga este gate; `dx audit`/`fetch`/`update` devolvem código 3 com
mensagem honesta de "não implementado". A fundação (`dx-security`, invocação de
`cargo-audit`, validade da base, tipos de veredicto) é o primeiro trabalho do
inc.2.
