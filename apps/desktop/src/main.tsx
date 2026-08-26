import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import App from "./App";
import { VisualFixtureApp } from "./dev/VisualFixtureApp";
import { DEFAULT_VISUAL_FIXTURE, parseVisualFixture } from "./dev/visualFixture";
import { LocaleProvider, resolveInitialLocale } from "./i18n/LocaleProvider";
import "@mission-control/ui/tokens.css";
import "@mission-control/ui/focus.css";
import "./app.css";

const requestedFixture = import.meta.env.DEV ? parseVisualFixture(window.location.search) : null;
const visualFixture = requestedFixture ?? (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)
  ? { ...DEFAULT_VISUAL_FIXTURE, locale: resolveInitialLocale() }
  : null);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <LocaleProvider initialLocale={visualFixture?.locale}>{visualFixture ? <VisualFixtureApp config={visualFixture} /> : <App />}</LocaleProvider>
  </StrictMode>,
);
