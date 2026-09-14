import type { Localized } from "../types";
import type { auth as AuthEn } from "../en/auth";

export const auth: Localized<typeof AuthEn> = {
  login: {
    tagline: "Mantenha os saves e savestates dos seus emuladores sincronizados entre dispositivos.",
    providerLabel: "Onde guardar seus saves",
    googleDriveHint: "Guardados numa pasta Slot2Sync no seu Drive",
    cloudHint: "Guardados numa pasta Slot2Sync na sua conta",
    localFolderHint: "Uma pasta neste computador ou num compartilhamento de rede",
    comingSoon: "Em breve",
    folderPathLabel: "Pasta",
    folderPathPlaceholder: "ex.: D:\\Slot2Sync ou \\\\servidor\\compartilhamento",
    chooseFolder: "Escolher pasta",
    folderPickerTitle: "Escolha onde o Slot2Sync guarda seus saves",
    deviceHint: "Aparece ao lado de cada save, para você saber qual dispositivo o alterou.",
    permissionNote:
      "O Slot2Sync <strong>não enxerga seus arquivos pessoais</strong>. Ele só lê e altera os arquivos que ele mesmo cria.",
    continueWith: "Continuar com {{provider}}",
    connectFolder: "Conectar pasta",
    waitingForBrowser: "Aguardando sua autorização no navegador…",
    connectingFolder: "Conectando à pasta…",
  },
  device: {
    nameLabel: "Nome deste dispositivo",
    namePlaceholder: "ex.: PC Gamer, Notebook",
  },
  account: {
    connected: "Conectado",
    disconnect: "Desconectar",
  },
};
