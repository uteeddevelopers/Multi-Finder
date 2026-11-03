import { useEffect } from "react";
import { useApp } from "./model/context";
import { Sidebar } from "./components/Sidebar";
import { Toolbar } from "./components/Toolbar";
import { StatusBar } from "./components/StatusBar";
import { AlertDialog, MoveDialog, TrashDialog } from "./components/Dialogs";
import { CollectionView } from "./screens/Collection";
import { DuplicatesView } from "./screens/Duplicates";
import { CompareView } from "./screens/Compare";
import { CategoriesView } from "./screens/Categories";
import { LeftoversView } from "./screens/Leftovers";

function isEditable(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  return el.isContentEditable || el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT";
}

function inPopup(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  return !!el?.closest?.('[role="menu"], [role="dialog"], [role="alertdialog"]');
}

/** Window-level shortcuts (Explorer conventions). Lists handle their own navigation keys. */
function useShortcuts() {
  const app = useApp();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const ctrl = e.ctrlKey || e.metaKey;
      const k = e.key.length === 1 ? e.key.toLowerCase() : e.key;
      // Never let the web view reload or open its own find bar.
      if (k === "F5" || (ctrl && !e.shiftKey && k === "r")) {
        e.preventDefault();
        if (!app.modalOpen && !app.isScanning) void app.rescan();
        return;
      }
      if (ctrl && k === "f") {
        e.preventDefault();
        if (!app.modalOpen) app.focusSearch();
        return;
      }
      if (app.modalOpen || inPopup(e.target)) return;
      if (ctrl && !e.shiftKey && k === "o") {
        e.preventDefault();
        void app.addFolderViaPicker();
      } else if (ctrl && e.shiftKey && k === "m") {
        e.preventDefault();
        app.moveOnCurrentScreen();
      } else if (ctrl && e.shiftKey && k === "r") {
        e.preventDefault();
        app.revealOnCurrentScreen();
      } else if (isEditable(e.target)) {
        return; // text fields keep their own Ctrl+Z / Delete
      } else if (ctrl && !e.shiftKey && k === "z") {
        e.preventDefault();
        void app.undo();
      } else if (k === "Delete") {
        e.preventDefault();
        app.trashOnCurrentScreen();
      }
    };
    const onContext = (e: MouseEvent) => {
      if (!isEditable(e.target)) e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("contextmenu", onContext);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("contextmenu", onContext);
    };
  }, [app]);
}

export function App() {
  const app = useApp();
  useShortcuts();
  return (
    <div className="app">
      <Sidebar />
      <main className="main">
        <Toolbar />
        <div className="content">
          {app.screen === "collection" && <CollectionView />}
          {app.screen === "duplicates" && <DuplicatesView />}
          {app.screen === "compare" && <CompareView />}
          {app.screen === "categories" && <CategoriesView />}
          {app.screen === "leftovers" && <LeftoversView />}
        </div>
        <StatusBar />
      </main>
      <TrashDialog />
      <MoveDialog />
      <AlertDialog />
    </div>
  );
}
