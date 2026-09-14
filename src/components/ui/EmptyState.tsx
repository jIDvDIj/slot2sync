import type { ReactNode } from "react";

import { Icon, type IconName } from "./Icon";

import "./EmptyState.css";

interface EmptyStateProps {
  icon: IconName;
  title: ReactNode;
  message?: ReactNode;
  /** The next step the person can take from here. */
  action?: ReactNode;
  compact?: boolean;
}

export function EmptyState({ icon, title, message, action, compact }: EmptyStateProps) {
  return (
    <div className="empty-state" data-compact={compact || undefined}>
      <span className="empty-state-icon">
        <Icon name={icon} size={compact ? 20 : 28} />
      </span>
      <p className="empty-state-title">{title}</p>
      {message ? <p className="empty-state-message">{message}</p> : null}
      {action ? <div className="empty-state-action">{action}</div> : null}
    </div>
  );
}
