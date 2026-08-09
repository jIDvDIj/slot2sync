# Scripts de tooling

Scripts de apoio ao desenvolvimento e ao CI. Nenhum deles participa do build do app —
são ferramentas de qualidade, release e manutenção do repositório.

> Setup completo (o que rodar após clonar, secrets do CI, etc.):
> [Configurar o tooling pendente](https://jidvdij.github.io/slot2sync-site/docs/dev/guias/setup-tooling-pendente/).

| Script                                           | O que faz                                                                                                                                                            | Quando rodar                                                                     |
| ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| [`install-hooks.sh`](./install-hooks.sh)         | Copia os hooks de `git-hooks/` para `.git/hooks/`                                                                                                                    | Uma vez por clone, após `git clone`                                              |
| [`git-hooks/commit-msg`](./git-hooks/commit-msg) | Hook que rejeita commits fora do padrão Conventional Commits (`tipo(escopo): descrição`)                                                                             | Automático em todo `git commit` (após instalado)                                 |
| [`check-i18n.mjs`](./check-i18n.mjs)             | Valida a paridade de chaves en ⇄ pt via `tsc --noEmit` (chave faltando = erro de tipo)                                                                               | `npm run i18n:check` — roda no CI em cada PR                                     |
| [`extract-i18n.mjs`](./extract-i18n.mjs)         | Audita chaves i18n: usadas mas não definidas (erro) e definidas mas nunca usadas (aviso); entende usos indiretos (`labelKey:`) e dinâmicos (``t(`errors.${code}`)``) | `npm run i18n:extract` — sob demanda, ao mexer em traduções                      |
| [`update-authors.sh`](./update-authors.sh)       | Regenera o arquivo `AUTHORS` a partir do git log (ordenado por commits, unifica identidades via `.mailmap`, filtra bots)                                             | `sh scripts/update-authors.sh` — quando entrar contribuidor novo                 |
| [`check-licenses.sh`](./check-licenses.sh)       | Checa as licenças das dependências Rust contra a política de `src-tauri/deny.toml` (requer `cargo install cargo-deny --locked`)                                      | `sh scripts/check-licenses.sh` — sob demanda; o CI roda o mesmo check em cada PR |

## Convenções

- **Shell**: POSIX `sh` puro, sem dependências — funcionam no WSL, Linux, macOS e Git Bash.
- **Node**: arquivos `.mjs` rodam com o Node do projeto (≥ 20); os que precisam compilar
  TypeScript usam o `esbuild` já presente via Vite (sem dependência extra).
- Scripts novos devem ser registrados nesta tabela e, se fizerem parte do fluxo de
  qualidade, ganhar um alias em `package.json` (`npm run ...`) e um job/step no CI.
