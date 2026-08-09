# Contribuindo com o Slot2Sync

Obrigado pelo interesse em contribuir. Antes de abrir um PR, leia isto.

## Fluxo de contribuição

1. Faça um fork do repositório e trabalhe numa branch própria.
2. Abra um Pull Request contra a `main`. Todo PR passa por CI (lint, format,
   testes Rust em Windows/Linux, clippy, cargo-audit, cargo-deny, cobertura)
   e exige aprovação do mantenedor antes de ser mesclado — inclusive PRs de
   colaboradores frequentes.
3. Siga [Conventional Commits](https://www.conventionalcommits.org/) nas
   mensagens (`tipo(escopo): descrição`), em inglês.
4. Rode `sh scripts/install-hooks.sh` uma vez após clonar, para instalar os
   hooks de validação de commit.

## Credenciais do Google OAuth — use as suas, não peça as de produção

O Slot2Sync se autentica no Google Drive via OAuth2 + PKCE. Para rodar e
testar o fluxo de login **localmente**, você precisa do seu **próprio**
OAuth Client ID do Google Cloud Console — nunca peça ou compartilhe o
`client_id`/`client_secret` de produção do projeto.

Passos:

1. Crie um projeto no [Google Cloud Console](https://console.cloud.google.com/).
2. Configure a tela de consentimento OAuth (tipo "Externo" está OK para
   testes — o escopo usado, `drive.file`, é não-sensível e não exige
   verificação do Google).
3. Crie uma credencial OAuth do tipo **Aplicativo da Web** (Web application) —
   é o mesmo tipo usado em produção, tanto para o fluxo desktop (redirect
   loopback em `127.0.0.1`) quanto para o mobile (redirect via Worker).
   Em "URIs de redirecionamento autorizados", adicione `http://localhost` (ou
   `http://127.0.0.1`) — o Google não exige a porta exata para esse host, o
   que permite a porta efêmera que o app abre a cada login.
4. Copie `.env.example` para `.env` na raiz do repositório e preencha
   `SLOT2SYNC_GOOGLE_CLIENT_ID` e `SLOT2SYNC_GOOGLE_CLIENT_SECRET` com os
   valores do seu client de teste. Não configure `SLOT2SYNC_TOKEN_PROXY_URL`
   nem `SLOT2SYNC_PROXY_SECRET` localmente — essas variáveis apontam para o
   Worker de produção e não são necessárias fora dele.
5. **Nunca** faça commit do seu `.env` — ele já está no `.gitignore`.

Se seu PR não envolve o fluxo de autenticação, você pode rodar o app sem
essas variáveis: ele inicia normalmente, só a conexão ao Drive fica
indisponível.

## O que NÃO fazer em um PR

- Não altere `.github/workflows/*.yml`, `src-tauri/build.rs`, `worker/`,
  `package.json` ou `Cargo.toml` "de passagem" dentro de um PR sobre outra
  coisa — essas áreas exigem revisão extra por afetarem o pipeline de
  build/release e o manuseio de credenciais.
- Não commite arquivos gerados localmente (`src-tauri/gen/android/`
  contém partes versionadas e partes ignoradas de propósito — veja o
  `.gitignore` antes de forçar a inclusão de algo).
- Não inclua segredos, tokens ou credenciais reais em código, testes ou
  mensagens de commit, mesmo de exemplo.
