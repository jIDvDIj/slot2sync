import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyAppearance, readStoredAppearance } from "./hooks/useAppearance";
import "./i18n";
import "./styles/tokens.css";
import "./styles/base.css";

// Before the first paint, so a forced appearance never flashes the OS one.
applyAppearance(readStoredAppearance());

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
