import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "./Button";
import { Icon, type IconName } from "./Icon";

import "./Banner.css";

export type BannerTone = "info" | "success" | "warning" | "danger";

const TONE_ICON: Record<BannerTone, IconName> = {
  info: "info",
  success: "checkCircle",
  warning: "warning",
  danger: "error",
};

export interface BannerProps {
  tone?: BannerTone;
  title?: ReactNode;
  children?: ReactNode;
  actions?: ReactNode;
  onDismiss?: () => void;
}

export function Banner({ tone = "info", title, children, actions, onDismiss }: BannerProps) {
  const { t } = useTranslation();
  return (
    <div className="banner" data-tone={tone} role={tone === "danger" ? "alert" : "status"}>
      <Icon name={TONE_ICON[tone]} size={18} className="banner-icon" />
      <div className="banner-content">
        {title ? <p className="banner-title">{title}</p> : null}
        {children ? <div className="banner-body">{children}</div> : null}
        {actions ? <div className="banner-actions">{actions}</div> : null}
      </div>
      {onDismiss ? (
        <Button
          variant="plain"
          size="small"
          icon="close"
          aria-label={t("common.dismiss")}
          onClick={onDismiss}
          className="banner-dismiss"
        />
      ) : null}
    </div>
  );
}
