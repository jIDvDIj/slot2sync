# Política de Segurança

## Reportando uma vulnerabilidade

Se você encontrou uma vulnerabilidade de segurança real no Slot2Sync (não
um bug comum — use as Issues normais para isso), **não abra uma issue
pública** detalhando como explorá-la.

Em vez disso, use o [GitHub Security Advisories](../../security/advisories/new)
deste repositório para reportar de forma privada. Isso notifica diretamente
o mantenedor sem expor detalhes de exploração publicamente antes de existir
uma correção.

Inclua, se possível:

- Passos para reproduzir.
- Impacto (o que um atacante consegue fazer).
- Versão/commit afetado.

## O que está fora de escopo

- Relatórios sobre o `client_id` OAuth do Google estar "exposto" — ele não é
  secreto por design (fluxo PKCE de app nativo, RFC 8252). O `client_secret`
  de produção nunca é embutido no binário; fica só no proxy Cloudflare
  Worker.
- Relatórios de dependências com CVE já cobertos pelo `cargo-audit`/
  `cargo-deny` do CI ou pelo Dependabot — esses já são monitorados
  automaticamente.

## Escopo

Este projeto sincroniza arquivos locais (saves/savestates de emuladores)
com o Google Drive via uma conta que o próprio usuário conecta. O modelo de
ameaça relevante é: proteção do refresh token no keyring/SQLite local,
integridade do fluxo OAuth (PKCE, validação de `state`), e o proxy Worker
que intermedia a troca de tokens sem expor o `client_secret`.
