import type { ComponentPropsWithRef } from "react";

import { cx } from "../../lib/cx";

import "./Switch.css";

interface SwitchProps extends Omit<
  ComponentPropsWithRef<"input">,
  "type" | "onChange" | "checked"
> {
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
}

/** Give it an `id` referenced by a visible label, or an `aria-label`. */
export function Switch({ checked, onCheckedChange, className, ...rest }: SwitchProps) {
  return (
    <span className={cx("switch", className)}>
      <input
        type="checkbox"
        role="switch"
        className="switch-input"
        checked={checked}
        onChange={(event) => onCheckedChange(event.target.checked)}
        {...rest}
      />
      <span className="switch-track" aria-hidden="true">
        <span className="switch-thumb" />
      </span>
    </span>
  );
}
