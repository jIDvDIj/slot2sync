import type { Localized } from "../types";
import type { common as CommonEn } from "../en/common";

export const common: Localized<typeof CommonEn> = {
  common: {
    close: "Fechar",
    add: "Adicionar",
    dismiss: "Dispensar",
  },
  app: {
    checkingConnection: "verificando conexão com o provedor remoto…",
    settings: "⚙ Configurações",
    emulators: "Emuladores",
    loading: "carregando…",
    noEmulators:
      "Nenhum emulador configurado. Use “Adicionar emulador” e selecione a pasta raiz do PPSSPP ou PCSX2.",
    switchToLightTheme: "☀ Tema claro",
    switchToDarkTheme: "🌙 Tema escuro",
  },
  panic: {
    title: "Algo falhou de forma inesperada",
    body: "Uma operação interna quebrou. O app continua rodando, mas a ação que a disparou pode não ter sido concluída. Os detalhes estão no arquivo de log.",
    at: "em",
  },
};
