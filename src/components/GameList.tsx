import { useTranslation } from "react-i18next";

import { formatBytes } from "../lib/format";
import type { SyncedGame } from "../types/ipc";
import { Badge } from "./ui/Badge";

/** Chaves i18n dos rótulos de categoria (reaproveitadas das configurações). */
const CATEGORY_LABEL = {
  saves: "settings.categories.saves",
  savestates: "settings.categories.savestates",
  config: "settings.categories.config",
} as const;

/**
 * Lista de jogos sincronizados de um emulador: nome legível (ou serial), as
 * categorias em que tem arquivos e o tamanho total.
 */
export function GameList({ games }: { games: SyncedGame[] }) {
  const { t } = useTranslation();

  if (games.length === 0) {
    return <p className="muted empty game-empty">{t("emulator.noGames")}</p>;
  }

  return (
    <ul className="game-list">
      {games.map((game) => (
        <li key={`${game.emulator}/${game.serial}`} className="game-row">
          <span className="game-name" title={game.serial}>
            {game.name ?? game.serial}
          </span>
          <span className="game-cats">
            {game.categories.map((category) => (
              <Badge key={category} tone="neutral" className="badge-cat">
                {t(CATEGORY_LABEL[category])}
              </Badge>
            ))}
          </span>
          <span className="game-size muted">{formatBytes(game.sizeBytes)}</span>
        </li>
      ))}
    </ul>
  );
}
