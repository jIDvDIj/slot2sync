# Objetivos e princípios do Slot2Sync

O que guia as decisões de projeto do Slot2Sync, em ordem de prioridade. A versão resumida
está no [`README.md`](./README.md#objetivos); este documento existe para o comentário mais
longo por trás de cada item.

## Objetivos de produto

1. **Seguro contra perda de dados.** O sync nunca deleta nada no Drive. Conflito entre duas
   máquinas é resolvido dando ao usuário a decisão final, nunca por sobrescrita silenciosa —
   e mesmo quando o Drive "vence" por padrão (primeiro sync, sem histórico ainda), o app faz
   backup local do que seria sobrescrito antes de agir.

2. **Suas credenciais ficam só suas.** O escopo OAuth é o mínimo necessário
   (`drive.file`) — o app só enxerga os arquivos que ele mesmo cria no Drive do usuário,
   nunca o resto da conta. O token de acesso fica no cofre de credenciais do sistema
   operacional, nunca em texto plano, e nunca cruza a fronteira entre o backend Rust e o
   frontend.

3. **Automático.** Depois de conectar a conta e apontar a pasta do emulador, não há mais
   nada para fazer — o app sincroniza sozinho, nos momentos certos (abrir o emulador, fechar
   o emulador, abrir e fechar o próprio app), sem exigir que o usuário lembre de fazer nada.

4. **Resiliente a falhas.** Sem internet, ou com o arquivo em uso pelo emulador? Vira uma
   pendência que é resolvida assim que possível — nunca um erro que trava o app ou exige
   intervenção manual para "destravar" o sync.

5. **Extensível.** O núcleo de sincronização não conhece nenhum emulador específico —
   suporte a um emulador novo é configuração declarativa (um catálogo de marcadores de
   filesystem), não reescrita de código.

## Princípios de engenharia

Como o código é construído para sustentar os objetivos acima:

1. **Backend "inteligente", frontend "burro".** Toda a lógica de negócio vive no Rust; o
   React só dispara comandos (`invoke`) e reage a eventos (`emit`). Evita estado duplicado
   entre as duas linguagens e mantém a superfície de ataque do frontend mínima.

2. **Núcleo agnóstico a emuladores.** O motor de sincronização opera sobre caminhos e
   categorias (`SyncTarget`), nunca sobre nomes de emulador — quem sabe o que é PPSSPP ou
   PCSX2 é o catálogo declarativo, não o motor.

3. **Segurança por padrão.** Tokens nunca cruzam a fronteira Rust↔TS; credenciais vivem no
   keychain nativo do sistema operacional; o escopo OAuth pedido é sempre o mínimo
   suficiente para a funcionalidade em questão.

4. **Não-destrutivo.** O sync nunca apaga arquivos no Drive — só adiciona e atualiza. Um
   arquivo sobrescrito localmente ainda pode ser recuperado pelo histórico de backup.

5. **Offline-first.** Falha de rede ou arquivo em uso nunca é um erro fatal — vira uma
   pendência persistida, retomada automaticamente no próximo gatilho de sync.

6. **Sem magic strings.** Nomes de pastas do Drive, chaves de segredo, triggers e
   parâmetros de runtime são constantes nomeadas num único lugar, não literais espalhados
   pelo código.
