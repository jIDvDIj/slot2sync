import type { Localized } from "../types";
import type { auth as AuthEn } from "../en/auth";

export const auth: Localized<typeof AuthEn> = {
  login: {
    tagline: "Sincronize saves, savestates e configs dos seus emuladores com o Google Drive.",
    permissionNote:
      "O Slot2Sync <strong>não acessa seus dados pessoais</strong>. Ele só consegue ver e modificar os arquivos que ele mesmo cria no seu Google Drive.",
    connecting: "Aguardando autorização no navegador…",
    connect: "Conectar ao Google Drive",
  },
  device: {
    nameLabel: "Nome deste dispositivo",
    namePlaceholder: "ex.: PC Gamer, Notebook",
  },
  account: {
    connected: "Conta Google conectada",
    disconnect: "Desconectar",
  },
};
