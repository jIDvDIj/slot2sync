import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import { formatRelativeTime } from "../../lib/time";
import type { SyncedGame } from "../../types/ipc";
import { FormSection } from "../ui/Form";
import { Icon } from "../ui/Icon";

const MAX_GAMES = 6;

interface RecentGamesProps {
  games: SyncedGame[];
  now: number;
  onOpenEmulator: (name: string) => void;
}

export function RecentGames({ games, now, onOpenEmulator }: RecentGamesProps) {
  const { t } = useTranslation();
  const recent = useMemo(
    () => [...games].sort((a, b) => b.lastSyncedAtMs - a.lastSyncedAtMs).slice(0, MAX_GAMES),
    [games],
  );

  if (recent.length === 0) return null;

  return (
    <FormSection title={t("overview.recentGames")}>
      <ul role="list" className="recent-games">
        {recent.map((game) => (
          <li key={`${game.emulator}/${game.serial}`}>
            <button
              type="button"
              className="recent-game"
              onClick={() => onOpenEmulator(game.emulator)}
            >
              <Icon name="gamepad" size={16} className="recent-game-icon" />
              <span className="recent-game-text">
                <span className="recent-game-name truncate" title={game.serial}>
                  {game.name ?? game.serial}
                </span>
                <span className="recent-game-detail truncate">
                  {t("overview.gameDetail", {
                    emulator: game.emulator,
                    when: formatRelativeTime(t, game.lastSyncedAtMs, now),
                  })}
                </span>
              </span>
              <Icon name="chevronRight" size={14} className="recent-game-chevron" />
            </button>
          </li>
        ))}
      </ul>
    </FormSection>
  );
}
