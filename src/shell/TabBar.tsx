import { useTranslation } from "react-i18next";

import { Icon, type IconName } from "../components/ui/Icon";
import type { Route } from "../lib/navigation";

interface TabBarProps {
  route: Route;
  onNavigate: (route: Route) => void;
  issueCount: number;
}

type TabName = "overview" | "activity" | "settings";

export function TabBar({ route, onNavigate, issueCount }: TabBarProps) {
  const { t } = useTranslation();
  const selected: TabName = route.name === "emulator" ? "overview" : route.name;

  const tabs: { name: TabName; icon: IconName; label: string }[] = [
    { name: "overview", icon: "grid", label: t("nav.overview") },
    { name: "activity", icon: "activity", label: t("nav.activity") },
    { name: "settings", icon: "settings", label: t("nav.settings") },
  ];

  return (
    <nav className="tab-bar" aria-label={t("nav.sidebar")}>
      {tabs.map((tab) => (
        <button
          key={tab.name}
          type="button"
          className="tab-bar-item"
          aria-current={selected === tab.name ? "page" : undefined}
          onClick={() => onNavigate({ name: tab.name })}
        >
          <span className="tab-bar-icon">
            <Icon name={tab.icon} size={22} />
            {tab.name === "activity" && issueCount > 0 ? (
              <span
                className="tab-bar-count tabular"
                aria-label={t("nav.issues", { count: issueCount })}
              >
                {issueCount}
              </span>
            ) : null}
          </span>
          <span className="tab-bar-label">{tab.label}</span>
        </button>
      ))}
    </nav>
  );
}
