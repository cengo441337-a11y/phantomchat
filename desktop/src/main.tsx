import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";
// i18next bootstrap — must run before App renders so the LanguageDetector
// has settled the active locale before the first useTranslation() call.
import "./i18n";
import { applyTheme } from "./components/ThemeSwitcher";

// Pre-apply the persisted theme BEFORE React mounts so the very first
// paint already uses the correct palette — no flash of cyberpunk on
// users who picked Soft Light or Corporate previously.
try {
  const stored = window.localStorage.getItem("phantomchat-theme");
  if (stored === "light" || stored === "corporate" || stored === "cyberpunk") {
    applyTheme(stored);
  } else {
    applyTheme("cyberpunk");
  }
} catch {
  applyTheme("cyberpunk");
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
