import type { ComponentPropsWithRef } from "react";

import { cx } from "../../lib/cx";
import { Icon, type IconName } from "./Icon";
import { Spinner } from "./Spinner";

import "./Button.css";

export type ButtonVariant =
  "prominent" | "bordered" | "plain" | "destructive" | "destructiveProminent";

export type ButtonSize = "small" | "regular" | "large";

export interface ButtonProps extends ComponentPropsWithRef<"button"> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: IconName;
  /** Replaces the icon with a spinner and blocks further clicks. */
  loading?: boolean;
  fullWidth?: boolean;
}

const ICON_SIZE: Record<ButtonSize, number> = { small: 14, regular: 16, large: 18 };

/** Icon-only buttons (no children) must receive an `aria-label`. */
export function Button({
  variant = "bordered",
  size = "regular",
  icon,
  loading = false,
  fullWidth = false,
  className,
  children,
  disabled,
  type = "button",
  ...rest
}: ButtonProps) {
  const iconOnly = icon !== undefined && (children === undefined || children === null);

  return (
    <button
      type={type}
      className={cx("button", className)}
      data-variant={variant}
      data-size={size}
      data-icon-only={iconOnly || undefined}
      data-full-width={fullWidth || undefined}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      {...rest}
    >
      {loading ? (
        <Spinner size="small" />
      ) : icon ? (
        <Icon name={icon} size={ICON_SIZE[size]} />
      ) : null}
      {iconOnly ? null : <span className="button-label">{children}</span>}
    </button>
  );
}
