import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";
// i18next bootstrap — must run before App renders so the LanguageDetector
// has settled the active locale before the first useTranslation() call.
import "./i18n";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
