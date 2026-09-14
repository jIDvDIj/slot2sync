import type { TFunction } from "i18next";

import { currentLocale } from "../i18n";

export function formatDateTime(ms: number): string {
  return new Date(ms).toLocaleString(currentLocale(), { dateStyle: "medium", timeStyle: "short" });
}

/** "just now", "5 min ago"… falling back to an absolute date after a day. */
export function formatRelativeTime(t: TFunction, atMs: number, nowMs: number): string {
  const seconds = Math.max(0, Math.round((nowMs - atMs) / 1000));
  if (seconds < 10) return t("time.justNow");
  if (seconds < 60) return t("time.secondsAgo", { count: seconds });
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return t("time.minutesAgo", { count: minutes });
  const hours = Math.round(minutes / 60);
  if (hours < 24) return t("time.hoursAgo", { count: hours });
  return formatDateTime(atMs);
}
