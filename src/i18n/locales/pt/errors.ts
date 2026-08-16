import type { Localized } from "../types";
import type { errors as ErrorsEn } from "../en/errors";

export const errors: Localized<typeof ErrorsEn> = {
  errors: {
    io: "erro de IO",
    database: "erro de banco de dados",
    network: "erro de rede",
    keyring: "erro no cofre de credenciais",
    serialization: "erro de serialização",
    auth: "erro de autenticação",
    emulator_not_detected: "emulador não reconhecido na pasta",
    emulator_exists: "já existe um emulador com este nome",
    file_busy: "arquivo em uso (modificado durante a leitura)",
    remote_not_found: "pasta ou arquivo não encontrado no provedor remoto",
    insufficient_disk_space: "espaço em disco insuficiente para o download",
    integrity: "falha na verificação de integridade da transferência",
    folder_not_mounted: "pasta não encontrada — o dispositivo está desconectado?",
    case_conflict: "dois arquivos com o mesmo nome em maiúsculas/minúsculas diferentes colidiriam",
    zip: "erro ao gerar o zip de diagnóstico",
    unexpected: "erro inesperado ao falar com o backend",
  },
};
