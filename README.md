# Slot2Sync

[![CI](https://github.com/jIDvDIj/slot2sync/actions/workflows/ci.yml/badge.svg)](https://github.com/jIDvDIj/slot2sync/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/jIDvDIj/slot2sync/graph/badge.svg)](https://codecov.io/gh/jIDvDIj/slot2sync)
[![License: GPL v3](https://img.shields.io/badge/license-GPLv3-blue.svg)](./LICENSE)
[![Latest release](https://img.shields.io/github/v/release/jIDvDIj/slot2sync)](https://github.com/jIDvDIj/slot2sync/releases/latest)

**Seus jogos, do ponto exato onde você parou — em qualquer máquina.**

O Slot2Sync é um aplicativo para computador que guarda automaticamente seus **saves,
savestates e configurações** de emuladores de retrogames no **Google Drive**. Você joga
no PC de casa, depois abre o mesmo jogo no notebook e continua de onde tinha parado — sem
copiar arquivos na mão, sem pendrive, sem se preocupar em perder progresso.

## A ideia

Quem joga emulador conhece a dor: o save do jogo fica preso numa pasta de uma máquina só.
Trocou de computador, formatou, ou só quer jogar no notebook no fim de semana? Lá se vai a
sincronia — ou começa a bagunça de copiar pastas para um pendrive e torcer para não
sobrescrever a versão certa.

O Slot2Sync resolve isso rodando discretamente em segundo plano e mantendo tudo guardado e
atualizado na sua conta do Google Drive, automaticamente.

## Objetivos

O que guia as decisões de projeto, em ordem de prioridade:

1. **Seguro contra perda de dados.**
2. **Suas credenciais ficam só suas.**
3. **Automático.**
4. **Resiliente a falhas.**
5. **Extensível.**

Esta é a versão resumida. Para mais contexto sobre cada objetivo e os princípios de
engenharia que os sustentam, veja o [`GOALS.md`](./GOALS.md) completo.

## Getting started

1. **Conecte sua conta Google.** Um clique, e pronto — o app só acessa o que ele mesmo cria
   no seu Drive, nada mais.
2. **Aponte a pasta do seu emulador.** O Slot2Sync reconhece sozinho qual emulador é.
3. **Esqueça que ele existe.** A partir daí tudo acontece sozinho. O app fica na bandeja do
   sistema, ao lado do relógio, e sincroniza nos momentos certos.

## O que ele faz por você

- **Sincroniza sozinho, na hora certa.** Quando você abre o emulador, ele baixa os saves
  mais recentes antes do jogo começar. Quando você fecha, envia o progresso novo para o
  Drive. Também sincroniza ao abrir o app e ao sair de vez.

- **Nunca apaga nada.** O Slot2Sync só adiciona e atualiza — seus arquivos no Drive estão
  seguros. Se duas máquinas mexeram no mesmo save, ele te dá o poder de decisão de qual save manter.

- **Funciona offline.** Sem internet ou com o jogo aberto na hora errada? Ele anota a
  pendência e sincroniza assim que der, sem dar erro nem atrapalhar.

- **Você escolhe o que sincronizar.** Dá para ligar ou desligar saves, savestates e
  configurações para cada emulador, individualmente — e também escolher quais momentos
  disparam a sincronização automática.

- **Avisa sem incomodar.** Notificações nativas mostram quando algo foi sincronizado ou deu
  problema — e você ajusta para receber todas, só os erros, ou nenhuma.

- **Combina com várias máquinas.** Cada computador ganha um nome, então você sempre sabe de
  onde veio cada save.

## Emuladores suportados

- **PPSSPP** (PlayStation Portable)
- **PCSX2** (PlayStation 2)

A estrutura foi pensada para crescer — novos emuladores podem ser adicionados sem mudar o
funcionamento do app.

## Seus dados e sua privacidade

- O Slot2Sync usa o acesso mínimo ao Google Drive: ele **só enxerga os arquivos que ele
  próprio cria**. O resto do seu Drive permanece invisível para o app.
- Tudo que ele guarda no Drive fica organizado numa pasta dedicada: `Slot2Sync`, com uma
  subpasta para cada emulador.

## Contribuindo e reportando problemas

Quer contribuir? Veja o [guia de contribuição](./CONTRIBUTING.md) — inclui como configurar
suas próprias credenciais de teste, sem depender das de produção.

Encontrou um bug? Abra uma [issue](https://github.com/jIDvDIj/slot2sync/issues).

Encontrou uma vulnerabilidade de segurança? **Não abra uma issue pública** — siga o processo
de divulgação responsável descrito em [`SECURITY.md`](./SECURITY.md).

## Build

O app é construído com **Tauri v2** (Rust + React). Resumo rápido:

```bash
npm install
npm run tauri build      # build completo (Windows/macOS/Linux)
cargo test --manifest-path src-tauri/Cargo.toml   # testes do backend
```

## Documentação

Procurando como **instalar e usar** o Slot2Sync? Veja o
[guia do usuário](https://jidvdij.github.io/slot2sync-site/docs/).

A documentação técnica — arquitetura, decisões de projeto e catálogo da fronteira
Rust↔TypeScript — mora no site, na aba
[Para desenvolvedores](https://jidvdij.github.io/slot2sync-site/docs/dev/). Antes de abrir um
PR, leia o [`CONTRIBUTING.md`](./CONTRIBUTING.md) e o [`CONDUCT.md`](./CONDUCT.md).

Todo o código é licenciado sob a [GPL-3.0-or-later](./LICENSE).
