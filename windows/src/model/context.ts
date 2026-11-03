import { createContext, useContext, useSyncExternalStore } from "react";
import type { AppModel } from "./app";

export const AppContext = createContext<AppModel | null>(null);

/** The app model; the calling component re-renders whenever the model changes. */
export function useApp(): AppModel {
  const app = useContext(AppContext);
  if (!app) throw new Error("useApp outside AppContext");
  useSyncExternalStore(app.subscribe, app.getVersion);
  return app;
}
