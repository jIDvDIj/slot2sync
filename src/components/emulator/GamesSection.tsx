import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { useNow } from "../../hooks/useNow";
import { formatBytes } from "../../lib/format";
import { formatDateTime, formatRelativeTime } from "../../lib/time";
import type { SyncedGame } from "../../types/ipc";
import { EmptyState } from "../ui/EmptyState";
import { FormRow, FormSection, TextField } from "../ui/Form";
import { StatusLabel } from "../ui/StatusLabel";
import { CATEGORY_LABEL } from "./categoryLabels";

import "./Emulator.css";

const SEARCH_THRESHOLD = 8;

export function GamesSection({ games }: { games: SyncedGame[] }) {
  const { t } = useTranslation();
  const now = useNow();
  const [query, setQuery] = useState("");

  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return [...games]
      .filter(
        (game) =>
          !needle ||
          game.serial.toLowerCase().includes(needle) ||
          (game.name ?? "").toLowerCase().includes(needle),
      )
      .sort((a, b) => b.lastSyncedAtMs - a.lastSyncedAtMs);
  }, [games, query]);

  return (
    <FormSection
      title={t("games.heading")}
      description={games.length > 0 ? t("games.count", { count: games.length }) : undefined}
    >
      {games.length > SEARCH_THRESHOLD ? (
        <div className="games-search">
          <TextField
            type="search"
            value={query}
            aria-label={t("games.searchLabel")}
            placeholder={t("games.searchPlaceholder")}
            onChange={(event) => setQuery(event.target.value)}
          />
        </div>
      ) : null}

      {games.length === 0 ? (
        <EmptyState
          compact
          icon="gamepad"
          title={t("games.empty")}
          message={t("games.emptyHint")}
        />
      ) : visible.length === 0 ? (
        <EmptyState compact icon="search" title={t("games.noMatches", { query: query.trim() })} />
      ) : (
        visible.map((game) => (
          <FormRow
            key={`${game.emulator}/${game.serial}`}
            icon="gamepad"
            label={game.name ?? game.serial}
            description={
              <span title={formatDateTime(game.lastSyncedAtMs)}>
                {[
                  game.name ? game.serial : null,
                  t("games.synced", { when: formatRelativeTime(t, game.lastSyncedAtMs, now) }),
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </span>
            }
            control={
              <>
                <span className="row-meta">
                  {game.categories.map((category) => (
                    <StatusLabel key={category}>{t(CATEGORY_LABEL[category])}</StatusLabel>
                  ))}
                </span>
                <span className="row-size">{formatBytes(game.sizeBytes)}</span>
              </>
            }
          />
        ))
      )}
    </FormSection>
  );
}
