import type { ReactNode } from "react";

import { Icon, type IconName } from "./Icon";

import "./StatusLabel.css";

export type StatusTone = "neutral" | "accent" | "success" | "warning" | "danger";

interface StatusLabelProps {
  tone?: StatusTone;
  /** Pair the tone with an icon: state is never carried by color alone. */
  icon?: IconName;
  spinning?: boolean;
  title?: string;
  children: ReactNode;
}

export function StatusLabel({
  tone = "neutral",
  icon,
  spinning,
  title,
  children,
}: StatusLabelProps) {
  return (
    <span className="status-label" data-tone={tone} title={title}>
      {icon ? <Icon name={icon} size={12} className={spinning ? "icon-spin" : undefined} /> : null}
      <span>{children}</span>
    </span>
  );
}
