// Browser component harness only. Never an entry in the production bundle.
import React from "react";
import { createRoot } from "react-dom/client";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "../../src/i18n/locales/en/translation.json";
import { SavedClips } from "../../src/components/settings/SavedClips";
await i18n
  .use(initReactI18next)
  .init({ lng: "en", resources: { en: { translation: en } } });
createRoot(document.getElementById("root")!).render(
  <SavedClips disabled={false} canRetry />,
);
