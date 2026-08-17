## O que este PR faz

<!-- Descreva a mudança e por quê. Se resolve uma issue, referencie-a (ex. "Closes #42"). -->

## Como testar

<!-- Passos manuais, se aplicável (ex. fluxo de UI que precisa ser conferido no Windows). -->

## Checklist

- [ ] Rodei `sh scripts/install-hooks.sh` neste clone (hook de validação de commit).
- [ ] Commits seguem [Conventional Commits](../CONTRIBUTING.md) em inglês (`tipo(escopo): descrição`).
- [ ] `cargo test`, `cargo clippy` e `cargo fmt --check` passam (mudanças em `src-tauri/`).
- [ ] `npm run lint`, `npm run format:check` e `npm run build` passam (mudanças em `src/`).
- [ ] Adicionei/atualizei testes cobrindo a mudança (o CI exige ≥80% de cobertura no patch via Codecov).
- [ ] Não modifiquei `.github/workflows/*.yml`, `src-tauri/build.rs`, `worker/`, `package.json` ou `Cargo.toml` sem necessidade — essas áreas afetam pipeline/credenciais e pedem atenção extra na revisão.
- [ ] Não incluí segredos, tokens ou credenciais (nem como exemplo) em código, testes ou mensagens de commit.

## Contexto adicional

<!-- Screenshots, decisões de design, trade-offs considerados, etc. -->
