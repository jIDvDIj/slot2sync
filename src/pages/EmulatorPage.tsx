import { useState } from "react";
import { useTranslation } from "react-i18next";

import { ConflictSection } from "../components/emulator/ConflictSection";
import { EmulatorProgress } from "../components/emulator/EmulatorProgress";
import { GamesSection } from "../components/emulator/GamesSection";
import { PendingSection } from "../components/emulator/PendingSection";
import { RemoveSection } from "../components/emulator/RemoveSection";
import { SummaryTiles } from "../components/emulator/SummaryTiles";
import { SyncOptionsSection } from "../components/emulator/SyncOptionsSection";
import { PageHeader } from "../components/ui/PageHeader";
import { StatusLabel } from "../components/ui/StatusLabel";
import { useEmulatorCategories } from "../hooks/useEmulatorCategories";
import { emulatorStatus } from "../lib/emulatorStatus";
import { STATUS_PRESENTATION } from "../lib/statusPresentation";
import type {
  Conflict,
  EmulatorProfile,
  PendingOp,
  SyncCategories,
  SyncedGame,
  SyncProgress,
} from "../types/ipc";

import "./EmulatorPage.css";

export interface EmulatorPageProps {
  profile: EmulatorProfile;
  running: boolean;
  /** Any sync in progress (this or another emulator). */
  syncing: boolean;
  /** Progress of the running sync; filter by `progress.emulator`. */
  progress: SyncProgress | null;
  trigger: string | null;
  conflicts: Conflict[];
  pendingOps: PendingOp[];
  games: SyncedGame[];
  /** Removes the emulator from sync and navigates back to the overview. */
  onRemove: (name: string) => Promise<void>;
  onConflictResolved: () => void;
  onPendingChanged: () => void;
  onSyncNow: () => Promise<void>;
}

export function EmulatorPage({
  profile,
  running,
  progress,
  conflicts,
  pendingOps,
  games,
  onRemove,
  onConflictResolved,
  onPendingChanged,
  onSyncNow,
}: EmulatorPageProps) {
  const { t } = useTranslation();
  const loadedCategories = useEmulatorCategories(profile.name);
  const [categoryOverride, setCategoryOverride] = useState<SyncCategories | null>(null);
  const categories = categoryOverride ?? loadedCategories;

  const ownProgress = progress?.emulator === profile.name ? progress : null;
  const status = emulatorStatus({
    running,
    syncing: ownProgress !== null,
    conflicts: conflicts.length,
    pendingOps,
    categories,
  });
  const presentation = STATUS_PRESENTATION[status.kind];

  return (
    <div className="emulator-page">
      <PageHeader
        title={profile.name}
        subtitle={
          <span className="emulator-path" title={profile.rootPath}>
            {profile.rootPath}
          </span>
        }
        accessory={
          <StatusLabel
            tone={presentation.tone}
            icon={presentation.icon}
            spinning={status.kind === "syncing"}
          >
            {t(presentation.labelKey)}
          </StatusLabel>
        }
      />

      <div className="emulator-sections">
        {conflicts.length > 0 ? (
          <ConflictSection
            emulator={profile.name}
            conflicts={conflicts}
            onResolved={onConflictResolved}
          />
        ) : null}

        {ownProgress ? <EmulatorProgress progress={ownProgress} /> : null}

        <SummaryTiles name={profile.name} />

        {pendingOps.length > 0 ? (
          <PendingSection ops={pendingOps} onSyncNow={onSyncNow} onChanged={onPendingChanged} />
        ) : null}

        <SyncOptionsSection
          profile={profile}
          categories={categories}
          onCategoriesChange={setCategoryOverride}
        />

        <GamesSection games={games} />

        <RemoveSection name={profile.name} onRemove={onRemove} />
      </div>
    </div>
  );
}
