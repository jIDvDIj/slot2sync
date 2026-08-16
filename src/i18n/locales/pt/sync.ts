import type { Localized } from "../types";
import type { sync as SyncEn } from "../en/sync";

export const sync: Localized<typeof SyncEn> = {
  sync: {
    syncNow: "Sincronizar agora",
    syncing: "Sincronizando…",
    justNow: "agora mesmo",
    secondsAgo: "há {{count}}s",
    minutesAgo: "há {{count}} min",
    hoursAgo: "há {{count}} h",
    queued: "pendentes {{count}}",
    failed: "falhas {{count}}",
    lastSync: "Último sync {{when}}",
    never: "Nenhuma sincronização ainda",
    backupBanner_one:
      "{{count}} arquivo local foi salvo em backup antes do primeiro sync (a versão remota venceu).",
    backupBanner_other:
      "{{count}} arquivos locais foram salvos em backup antes do primeiro sync (a versão remota venceu).",
    openBackupFolder: "Abrir pasta de backup",
    lastSyncError: "Falha no último sync{{emulator}}: {{message}}",
    autoPreGame: "Baixando saves frescos antes do jogo…",
    autoPostGame: "Enviando os saves da sessão…",
  },
  emulator: {
    conflictBadge: "conflito",
    running: "em execução",
    idle: "parado",
    syncing: "sincronizando",
    pendingBadge_one: "{{count}} pendente",
    pendingBadge_other: "{{count}} pendentes",
    resolveConflict_one: "Resolver conflito",
    resolveConflict_other: "Resolver conflito ({{count}})",
    removing: "Removendo…",
    remove: "Remover",
    games_one: "▸ {{count}} jogo",
    games_other: "▸ {{count}} jogos",
    hideGames: "▾ Ocultar jogos",
    noGames: "nenhum jogo sincronizado ainda",
    statsLine: "Último sync {{when}} · ↑{{up}} · ↓{{down}}",
  },
  pending: {
    title: "Arquivos pendentes — {{emulator}}",
    intro:
      "Estes arquivos não puderam ser transferidos (problema de rede ou arquivo em uso). Eles são retentados automaticamente a cada sync e serão sincronizados assim que o problema for resolvido.",
    empty: "Nenhum arquivo pendente.",
    upload: "envio",
    download: "download",
    attempts_one: "{{count}} tentativa",
    attempts_other: "{{count}} tentativas",
    retryNow: "Tentar novamente agora",
    retrying: "Sincronizando…",
    dead: "desistiu após muitas tentativas",
    retryFile: "Retentar este arquivo",
    bumpFile: "↑ Priorizar",
    prioritized: "priorizado",
  },
  conflict: {
    title: "Conflito — {{emulator}}",
    intro:
      "Estes arquivos mudaram neste dispositivo e no armazenamento remoto desde o último sync. Escolha qual versão manter — o sync deste emulador está pausado até a resolução. A versão descartada localmente é salva em backup.",
    thisDevice: "Este dispositivo",
    remote: "Remoto",
    openCopy: "Mostrar cópia local na pasta",
    keepLocal: "Manter local",
    keepRemote: "Manter remoto",
  },
};
