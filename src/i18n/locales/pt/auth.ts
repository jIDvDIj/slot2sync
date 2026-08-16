import type { Localized } from "../types";
import type { auth as AuthEn } from "../en/auth";

export const auth: Localized<typeof AuthEn> = {
  login: {
    tagline: "Sincronize saves, savestates e configs dos seus emuladores com a nuvem.",
    permissionNote:
      "O Slot2Sync <strong>não acessa seus dados pessoais</strong>. Ele só consegue ver e modificar os arquivos que ele mesmo cria no provedor escolhido.",
    connecting: "Aguardando autorização no navegador…",
    connectFolder: "Conectar pasta",
    connect: "Conectar ao {{provider}}",
    providerLabel: "Provedor de storage",
    providerLocalFolder: "Pasta local/rede",
    comingSoon: "em breve",
    folderPathLabel: "Caminho da pasta",
    folderPathPlaceholder: "ex.: D:\\Slot2Sync ou \\\\servidor\\compartilhamento",
    selectFolder: "Selecionar pasta…",
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
