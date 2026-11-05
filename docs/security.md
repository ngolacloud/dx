# Política de segurança do DX

> Estado: o **gate** descrito aqui é o desenho do incremento 2
> ([ADR-0005](adr/0005-gate-seguranca.md)). O incremento 1 **não** o liga —
> `dx audit`/`fetch`/`update` devolvem exit 3. Este documento fixa o contrato
> para que a implementação não derive.

## O que o gate garante (quando ligado)

- **Download ≠ autorização.** Bytes podem chegar ao cache do Cargo; nada
  não-aprovado segue para build/run/install **através do DX**.
- Avaliação de **todas** as vulnerabilidades conhecidas aplicáveis (directas e
  transitivas) antes de confirmar `add` e antes de compilar/correr.
- Categorias distintas: `vulnerability`, `malware` (bloqueio duro), `yanked`,
  `unmaintained`, `integrity-failure` (bloqueio duro), `source-not-allowed`
  (bloqueio duro).
- Base de advisories com **validade** (baseline 24 h em dev; refresh obrigatório
  antes de release). Base expirada → **sem** autorização de novas adições/
  builds/release; offline não converte indisponibilidade em sucesso.

## O que o gate **não** garante

- Ausência de CVE **não** prova ausência de código malicioso.
- Git/path privados sem advisory → "sem correspondência conhecida", nunca
  "comprovadamente seguro".
- **Invocar `cargo` directamente contorna o gate DX.** A aplicação da política
  da organização depende de CI obrigatória e de um ambiente de build controlado.
  O DX é opcional por desenho.

## Exemplo de política (`dx.toml`, futuro)

```toml
[security]
# Validade máxima da base de advisories.
max_db_age_dev = "24h"
refresh_before_release = true

[security.sources]
# Sem fallback de privado para público (dependency confusion).
allow = ["crates.io", "empresa"]
private = ["empresa"]

[security.policy]
vulnerability = "block"      # advisories RUSTSEC, com ou sem CVE
malware       = "block"      # bloqueio duro
integrity     = "block"      # checksum/arquivo inválido: bloqueio duro
yanked        = "warn"
unmaintained  = "warn"

# Excepções: identificam advisory/package/version, motivo, responsável,
# evidência e expiração. Sem --skip-security genérico, sem ignore global.
[[security.exception]]
advisory = "RUSTSEC-0000-0000"
package  = "exemplo"
version  = "1.2.3"
reason   = "falso positivo confirmado em <link>"
owner    = "walter"
expires  = "2026-12-01"
```

A política de uma organização não pode ser enfraquecida por um `dx.toml` vindo
de um repositório desconhecido; o merge de política é restritivo
(o mais apertado vence) e as excepções são validadas na CI de release.
