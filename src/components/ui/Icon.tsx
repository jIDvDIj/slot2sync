import type { ComponentPropsWithoutRef, ReactElement } from "react";

import { cx } from "../../lib/cx";

const ICONS = {
  activity: <path d="M3 12h4l3-7.5 4 15 3-7.5h4" />,
  archive: (
    <>
      <path d="M4 5h16v4H4z" />
      <path d="M5.5 9v9.5A1.5 1.5 0 0 0 7 20h10a1.5 1.5 0 0 0 1.5-1.5V9" />
      <path d="M10 13h4" />
    </>
  ),
  arrowDown: <path d="M12 5v14M6 13l6 6 6-6" />,
  arrowUp: <path d="M12 19V5M6 11l6-6 6 6" />,
  check: <path d="M5 12.5l4.5 4.5L19 7.5" />,
  checkCircle: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M8 12.2l2.8 2.8 5.4-5.5" />
    </>
  ),
  chevronDown: <path d="M6 9.5l6 6 6-6" />,
  chevronLeft: <path d="M14.5 6l-6 6 6 6" />,
  chevronRight: <path d="M9.5 6l6 6-6 6" />,
  chevronUpDown: <path d="M8 9.5l4-4 4 4M8 14.5l4 4 4-4" />,
  clock: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7.5V12l3 2" />
    </>
  ),
  close: <path d="M6.5 6.5l11 11M17.5 6.5l-11 11" />,
  cloud: <path d="M7 18.5h10.5a4 4 0 0 0 .6-7.95A6 6 0 0 0 6.6 9.1 4.7 4.7 0 0 0 7 18.5z" />,
  copy: (
    <>
      <path d="M9 9h10.5v11H9z" />
      <path d="M15 9V4.5H4.5v11H9" />
    </>
  ),
  disk: (
    <>
      <path d="M4 14.5l2.3-8A2 2 0 0 1 8.2 5h7.6a2 2 0 0 1 1.9 1.5l2.3 8V18a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 18z" />
      <path d="M4 14.5h16M16.5 17h.01" />
    </>
  ),
  document: (
    <>
      <path d="M7 3.5h7l4 4V20a.5.5 0 0 1-.5.5h-10A.5.5 0 0 1 6.5 20V4a.5.5 0 0 1 .5-.5z" />
      <path d="M14 3.5V8h4M9.5 12.5h5M9.5 16h5" />
    </>
  ),
  error: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M9.2 9.2l5.6 5.6M14.8 9.2l-5.6 5.6" />
    </>
  ),
  external: (
    <path d="M14 4h6v6M20 4l-8.5 8.5M18 14v4.5a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 4 18.5v-11A1.5 1.5 0 0 1 5.5 6H10" />
  ),
  folder: (
    <path d="M3.5 7.5a2 2 0 0 1 2-2h4l2 2h7a2 2 0 0 1 2 2V17a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z" />
  ),
  gamepad: (
    <>
      <path d="M7 8.5h10a4.5 4.5 0 0 1 4.4 5.5l-.7 3a2.6 2.6 0 0 1-4.5 1.1L14.4 16H9.6l-1.8 2.1a2.6 2.6 0 0 1-4.5-1.1l-.7-3A4.5 4.5 0 0 1 7 8.5z" />
      <path d="M8 11v3.5M6.25 12.75h3.5M15.5 12h.01M17.5 14h.01" />
    </>
  ),
  globe: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9c-2.5-2.6-3.8-5.6-3.8-9S9.5 5.6 12 3z" />
    </>
  ),
  grid: <path d="M4.5 4.5h6v6h-6zM13.5 4.5h6v6h-6zM4.5 13.5h6v6h-6zM13.5 13.5h6v6h-6z" />,
  info: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 11v5.5M12 7.75h.01" />
    </>
  ),
  laptop: <path d="M5 6.5A1.5 1.5 0 0 1 6.5 5h11A1.5 1.5 0 0 1 19 6.5V15H5zM3 18.5h18" />,
  bell: <path d="M6 16v-5a6 6 0 1 1 12 0v5l1.5 2h-15zM10 20.5a2 2 0 0 0 4 0" />,
  lock: (
    <>
      <path d="M6 11h12v9H6z" />
      <path d="M8.5 11V8a3.5 3.5 0 0 1 7 0v3" />
    </>
  ),
  pause: <path d="M9 6.5v11M15 6.5v11" />,
  play: <path d="M8 5.5v13l10.5-6.5z" />,
  plus: <path d="M12 5v14M5 12h14" />,
  power: <path d="M12 3.5v8M7.2 6.6a7 7 0 1 0 9.6 0" />,
  restore: <path d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3L4.5 9M4.5 4.5V9H9" />,
  search: (
    <>
      <circle cx="11" cy="11" r="6.5" />
      <path d="M20 20l-4.3-4.3" />
    </>
  ),
  settings: (
    <>
      <path d="M4 7h9.5M18.5 7H20M4 17h3.5M12.5 17H20" />
      <circle cx="16" cy="7" r="2.5" />
      <circle cx="10" cy="17" r="2.5" />
    </>
  ),
  sidebar: (
    <>
      <path d="M4 5.5h16v13H4z" />
      <path d="M9.5 5.5v13" />
    </>
  ),
  sync: (
    <>
      <path d="M19.5 10.5A7.5 7.5 0 0 0 6 7.1L4.5 8.5" />
      <path d="M4.5 4.5v4h4" />
      <path d="M4.5 13.5A7.5 7.5 0 0 0 18 16.9l1.5-1.4" />
      <path d="M19.5 19.5v-4h-4" />
    </>
  ),
  trash: (
    <path d="M4.5 7h15M9.5 7V4.5h5V7M6.5 7l.9 12a1.5 1.5 0 0 0 1.5 1.4h6.2a1.5 1.5 0 0 0 1.5-1.4l.9-12" />
  ),
  warning: (
    <>
      <path d="M10.3 4.3a2 2 0 0 1 3.4 0l7.3 12.6a2 2 0 0 1-1.7 3.1H4.7A2 2 0 0 1 3 16.9z" />
      <path d="M12 9.5v4M12 16.75h.01" />
    </>
  ),
} satisfies Record<string, ReactElement>;

export type IconName = keyof typeof ICONS;

interface IconProps extends Omit<ComponentPropsWithoutRef<"svg">, "name"> {
  name: IconName;
  size?: number;
  /** Accessible name; without it the icon is decorative and hidden from assistive tech. */
  label?: string;
}

export function Icon({ name, size = 16, label, className, ...rest }: IconProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={cx("icon", className)}
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      focusable="false"
      {...rest}
    >
      {ICONS[name]}
    </svg>
  );
}
