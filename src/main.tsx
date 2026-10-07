import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { I18nProvider, initLanguage } from "./lib/i18n";
import { applyTheme, readThemePref } from "./lib/theme";
import "./globals.css";

void Promise.all([initLanguage(), applyTheme(readThemePref())]).finally(() =>
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <I18nProvider>
        <App />
      </I18nProvider>
    </StrictMode>
  )
);
