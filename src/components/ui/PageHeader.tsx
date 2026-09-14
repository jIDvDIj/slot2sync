import type { ReactNode } from "react";

import "./PageHeader.css";

interface PageHeaderProps {
  title: ReactNode;
  subtitle?: ReactNode;
  /** Controls that act on the whole page, aligned to the title's baseline. */
  accessory?: ReactNode;
}

/** The page's only <h1>; the toolbar echoes the title once this scrolls away. */
export function PageHeader({ title, subtitle, accessory }: PageHeaderProps) {
  return (
    <header className="page-header">
      <div className="page-header-text">
        <h1 className="page-title">{title}</h1>
        {subtitle ? <p className="page-subtitle">{subtitle}</p> : null}
      </div>
      {accessory ? <div className="page-header-accessory">{accessory}</div> : null}
    </header>
  );
}
