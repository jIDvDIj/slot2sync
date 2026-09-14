import type { IconName } from "../components/ui/Icon";
import type { StatusTone } from "../components/ui/StatusLabel";
import type { EmulatorStatus } from "./emulatorStatus";

export const STATUS_PRESENTATION = {
  conflict: { tone: "danger", icon: "warning", labelKey: "status.conflict" },
  syncing: { tone: "accent", icon: "sync", labelKey: "status.syncing" },
  failed: { tone: "danger", icon: "error", labelKey: "status.failed" },
  pending: { tone: "warning", icon: "clock", labelKey: "status.pending" },
  paused: { tone: "neutral", icon: "pause", labelKey: "status.paused" },
  running: { tone: "success", icon: "play", labelKey: "status.running" },
  idle: { tone: "neutral", icon: "check", labelKey: "status.idle" },
} as const satisfies Record<
  EmulatorStatus["kind"],
  { tone: StatusTone; icon: IconName; labelKey: string }
>;
