// i18n bootstrap — German + English bundles loaded at module-import time.
//
// Detection order: localStorage (`i18nextLng` key, set by the manual toggle
// in SettingsPanel) → browser `navigator.language` → fallback `en`. The
// localStorage cache lets the user override the auto-detected locale on
// any subsequent launch without re-clicking through Settings.

import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import LanguageDetector from "i18next-browser-languagedetector";

import en from "./en.json";
import de from "./de.json";

void i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources: {
      en: { translation: en },
      de: { translation: de },
    },
    fallbackLng: "en",
    supportedLngs: ["en", "de"],
    nonExplicitSupportedLngs: true,
    interpolation: {
      // React already escapes by default — no need for i18next's escape pass.
      escapeValue: false,
    },
    detection: {
      order: ["localStorage", "navigator", "htmlTag"],
      caches: ["localStorage"],
      lookupLocalStorage: "i18nextLng",
    },
  });

export default i18n;
