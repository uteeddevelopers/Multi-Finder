import { StrictMode, useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { FluentProvider, webDarkTheme, webLightTheme, type Theme } from "@fluentui/react-components";
import { pick } from "./api/backend";
import type { LaunchArgs, Settings } from "./api/types";
import { currentLang, setLanguage, type Lang } from "./i18n";
import { AppModel, errorText } from "./model/app";
import { AppContext } from "./model/context";
import { ContextMenuProvider } from "./components/ContextMenu";
import { App } from "./App";
import "./styles.css";

const CJK_FONTS: Record<Lang, string> = {
  ko: '"Malgun Gothic", "맑은 고딕", "Noto Sans CJK KR", ',
  ja: '"Yu Gothic UI", "Meiryo UI", "Noto Sans CJK JP", ',
  "zh-Hans": '"Microsoft YaHei UI", "Noto Sans CJK SC", ',
  en: "",
};

function withFonts(theme: Theme, lang: Lang): Theme {
  return {
    ...theme,
    fontFamilyBase: `"Segoe UI Variable Text", "Segoe UI Variable", "Segoe UI", ${CJK_FONTS[lang]}system-ui, -apple-system, sans-serif`,
    fontFamilyMonospace: '"Cascadia Mono", Consolas, "Courier New", monospace',
  };
}

function useDarkMode(): boolean {
  const query = "(prefers-color-scheme: dark)";
  const [dark, setDark] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const mq = window.matchMedia(query);
    const on = () => setDark(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  return dark;
}

function Root({ app }: { app: AppModel }) {
  const dark = useDarkMode();
  const theme = useMemo(() => withFonts(dark ? webDarkTheme : webLightTheme, currentLang()), [dark]);
  useEffect(() => {
    document.documentElement.dataset.theme = dark ? "dark" : "light";
    document.documentElement.style.colorScheme = dark ? "dark" : "light";
  }, [dark]);
  return (
    <FluentProvider theme={theme}>
      <AppContext.Provider value={app}>
        <ContextMenuProvider>
          <App />
        </ContextMenuProvider>
      </AppContext.Provider>
    </FluentProvider>
  );
}

async function boot() {
  const backend = await pick();
  let settings: Settings;
  let startupError: string | null = null;
  try {
    settings = await backend.getSettings();
  } catch (e) {
    startupError = errorText(e);
    settings = { folders: [], locale: navigator.language, dataDir: "", version: "" };
  }
  setLanguage(settings.locale);
  const app = new AppModel(backend, settings);
  const launch: LaunchArgs = await backend.getLaunchArgs().catch(() => ({ run: false }));
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <Root app={app} />
    </StrictMode>,
  );
  await app.start(launch);
  if (startupError) app.showError(startupError);
}

void boot();
