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
};
