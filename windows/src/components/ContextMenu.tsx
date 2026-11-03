// Right-click menus at the pointer, shared by every list (one Fluent Menu anchored to a virtual element).
import { createContext, useCallback, useContext, useMemo, useState, type ReactElement, type ReactNode } from "react";
import { Menu, MenuDivider, MenuGroupHeader, MenuItem, MenuList, MenuPopover, MenuTrigger } from "@fluentui/react-components";

export type MenuSpec =
  | { label: string; onClick?: () => void; disabled?: boolean; icon?: ReactElement; danger?: boolean; submenu?: MenuSpec[] }
  | { divider: true }
  | { header: string };

type Opener = (pos: { clientX: number; clientY: number }, items: MenuSpec[]) => void;

const MenuContext = createContext<Opener>(() => {});

export function useContextMenu(): Opener {
  return useContext(MenuContext);
}

function renderItems(items: MenuSpec[], close: () => void): ReactNode[] {
  const out: ReactNode[] = [];
  items.forEach((it, i) => {
    if ("divider" in it) {
      if (i > 0 && i < items.length - 1 && !("divider" in items[i - 1])) out.push(<MenuDivider key={`d${i}`} />);
      return;
    }
    if ("header" in it) {
      out.push(<MenuGroupHeader key={`h${i}`}>{it.header}</MenuGroupHeader>);
      return;
    }
    if (it.submenu) {
      out.push(
        <Menu key={`s${i}`}>
          <MenuTrigger disableButtonEnhancement>
            <MenuItem icon={it.icon} disabled={it.disabled}>
              {it.label}
            </MenuItem>
          </MenuTrigger>
          <MenuPopover>
            <MenuList>{renderItems(it.submenu, close)}</MenuList>
          </MenuPopover>
        </Menu>,
      );
      return;
    }
    out.push(
      <MenuItem
        key={`i${i}`}
        icon={it.icon}
        disabled={it.disabled}
        className={it.danger ? "menu-danger" : undefined}
        onClick={() => {
          close();
          it.onClick?.();
        }}
      >
        {it.label}
      </MenuItem>,
    );
  });
  return out;
}

export function ContextMenuProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<{ x: number; y: number; items: MenuSpec[]; id: number } | null>(null);
  const open = useCallback<Opener>((pos, items) => {
    if (!items.length) return;
    setState({ x: pos.clientX, y: pos.clientY, items, id: Date.now() });
  }, []);
  const close = useCallback(() => setState(null), []);
  const target = useMemo(
    () =>
      state
        ? {
            getBoundingClientRect: () => ({ x: state.x, y: state.y, top: state.y, left: state.x, bottom: state.y, right: state.x, width: 0, height: 0 }),
          }
        : undefined,
    [state],
  );
  return (
    <MenuContext.Provider value={open}>
      {children}
      {state && (
        <Menu
          key={state.id}
          open
          onOpenChange={(_, data) => {
            if (!data.open) close();
          }}
          positioning={{ target, position: "below", align: "start", offset: 2 }}
        >
          <MenuPopover>
            <MenuList>{renderItems(state.items, close)}</MenuList>
          </MenuPopover>
        </Menu>
      )}
    </MenuContext.Provider>
  );
}
