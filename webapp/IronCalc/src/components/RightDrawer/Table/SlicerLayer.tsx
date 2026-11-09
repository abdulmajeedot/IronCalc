import type { Model, SlicerEntry, TableFilterItem } from "@ironcalc/wasm";
import { FilterX, ListChecks, Settings2, Trash2 } from "lucide-react";
import {
  type CSSProperties,
  forwardRef,
  type PointerEvent as ReactPointerEvent,
  useEffect,
  useImperativeHandle,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { Menu } from "../../Menu/Menu";
import { MenuDivider } from "../../Menu/MenuDivider";
import { MenuItem } from "../../Menu/MenuItem";
import { Alert } from "../../Modal";
import {
  headerColumnWidth,
  headerRowHeight,
} from "../../WorksheetCanvas/constants";
import type WorksheetCanvas from "../../WorksheetCanvas/worksheetCanvas";
import { messageOf } from "./TableMenus";
import "./table.css";

const MIN_WIDTH = 60;
const MIN_HEIGHT = 40;

const HANDLES = ["nw", "n", "ne", "e", "se", "s", "sw", "w"] as const;
type Handle = (typeof HANDLES)[number];

interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface SlicerLayerHandle {
  update: () => void;
}

interface SlicerLayerProps {
  model: Model;
  getCanvas: () => WorksheetCanvas | null;
  canEdit: boolean;
  refresh: () => void;
  onShowSettings: (name: string) => void;
  onSelect: (name: string) => void;
}

// The colour a slicer style is built around, and whether its buttons are solid.
export function slicerColors(
  model: Model,
  style: string,
): { accent: string; solid: boolean } {
  const theme = model.getTheme();
  const accents = [
    theme.accent1,
    theme.accent2,
    theme.accent3,
    theme.accent4,
    theme.accent5,
    theme.accent6,
  ];
  const match = /^SlicerStyle(Light|Dark|Other)(\d)$/.exec(style);
  if (!match) {
    return { accent: theme.accent1, solid: false };
  }
  const number = Number(match[2]);
  if (match[1] === "Other") {
    return { accent: number === 1 ? theme.dk2 : "#7F7F7F", solid: false };
  }
  return {
    accent: accents[(number - 1) % accents.length],
    solid: match[1] === "Dark",
  };
}

// Slicers float over the sheet as ordinary elements. The worksheet calls
// `update` after each paint, which keeps them on their cells while scrolling.
export const SlicerLayer = forwardRef<SlicerLayerHandle, SlicerLayerProps>(
  function SlicerLayer(
    { model, getCanvas, canEdit, refresh, onShowSettings, onSelect },
    ref,
  ) {
    const { t } = useTranslation();
    const [, setTick] = useState(0);
    const [selected, setSelected] = useState<string | null>(null);
    const [dragging, setDragging] = useState<{
      name: string;
      box: Box;
    } | null>(null);
    const [menu, setMenu] = useState<{
      name: string;
      x: number;
      y: number;
    } | null>(null);
    const [error, setError] = useState("");

    useImperativeHandle(
      ref,
      () => ({ update: () => setTick((tick) => tick + 1) }),
      [],
    );

    useEffect(() => {
      if (selected === null) {
        return;
      }
      const clearSelection = (event: PointerEvent) => {
        const target = event.target as HTMLElement | null;
        if (!target?.closest(".ic-slicer, .ic-menu, .ic-right-drawer")) {
          setSelected(null);
        }
      };
      window.addEventListener("pointerdown", clearSelection);
      return () => window.removeEventListener("pointerdown", clearSelection);
    }, [selected]);

    const canvas = getCanvas();
    if (!canvas) {
      return null;
    }
    const sheet = model.getSelectedSheet();
    const entries = model.getSlicers(sheet);
    const alert = (
      <Alert
        open={error !== ""}
        title={t("table.error_title")}
        message={error}
        onClose={() => setError("")}
      />
    );
    if (entries.length === 0) {
      return alert;
    }

    const run = (action: () => void) => {
      try {
        action();
        refresh();
      } catch (e) {
        setError(messageOf(e));
      }
    };

    const boxOf = (entry: SlicerEntry): Box => {
      if (dragging?.name === entry.slicer.name) {
        return dragging.box;
      }
      const { slicer } = entry;
      const [x, y] = canvas.getCoordinatesByCell(slicer.row, slicer.column);
      return {
        left: x + slicer.offset_x - headerColumnWidth,
        top: y + slicer.offset_y - headerRowHeight,
        width: slicer.width,
        height: slicer.height,
      };
    };

    const beginDrag = (
      event: ReactPointerEvent,
      entry: SlicerEntry,
      handle: Handle | "move",
    ) => {
      if (!canEdit || entry.slicer.locked || event.button !== 0) {
        return;
      }
      event.preventDefault();
      const start = { x: event.clientX, y: event.clientY };
      const origin = boxOf(entry);
      let latest = origin;
      let moved = false;
      const onMove = (move: PointerEvent) => {
        const dx = move.clientX - start.x;
        const dy = move.clientY - start.y;
        if (Math.abs(dx) + Math.abs(dy) > 2) {
          moved = true;
        }
        const box = { ...origin };
        if (handle === "move") {
          box.left += dx;
          box.top += dy;
        } else {
          if (handle.includes("e")) {
            box.width = Math.max(MIN_WIDTH, origin.width + dx);
          }
          if (handle.includes("s")) {
            box.height = Math.max(MIN_HEIGHT, origin.height + dy);
          }
          if (handle.includes("w")) {
            box.width = Math.max(MIN_WIDTH, origin.width - dx);
            box.left = origin.left + origin.width - box.width;
          }
          if (handle.includes("n")) {
            box.height = Math.max(MIN_HEIGHT, origin.height - dy);
            box.top = origin.top + origin.height - box.height;
          }
        }
        latest = box;
        setDragging({ name: entry.slicer.name, box });
      };
      const onUp = () => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        setDragging(null);
        if (!moved) {
          return;
        }
        const x = Math.max(latest.left, 0) + headerColumnWidth;
        const y = Math.max(latest.top, 0) + headerRowHeight;
        const cell = canvas.getCellByCoordinates(x, y);
        if (!cell) {
          return;
        }
        const [cellX, cellY] = canvas.getCoordinatesByCell(
          cell.row,
          cell.column,
        );
        run(() =>
          model.updateSlicer(entry.slicer.name, {
            ...entry.slicer,
            row: cell.row,
            column: cell.column,
            offset_x: Math.max(x - cellX, 0),
            offset_y: Math.max(y - cellY, 0),
            width: latest.width,
            height: latest.height,
          }),
        );
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    };

    const toggleItem = (
      entry: SlicerEntry,
      item: TableFilterItem,
      additive: boolean,
    ) => {
      const on = entry.items.filter((i) => i.visible).map((i) => i.label);
      let next: string[];
      if (additive || entry.slicer.multi_select) {
        if (!entry.filtered) {
          next = [item.label];
        } else if (on.includes(item.label)) {
          next = on.filter((label) => label !== item.label);
        } else {
          next = [...on, item.label];
        }
      } else if (entry.filtered && on.length === 1 && on[0] === item.label) {
        next = [];
      } else {
        next = [item.label];
      }
      run(() => model.setSlicerSelection(entry.slicer.name, next));
    };

    const setMultiSelect = (entry: SlicerEntry) =>
      run(() =>
        model.updateSlicer(entry.slicer.name, {
          ...entry.slicer,
          multi_select: !entry.slicer.multi_select,
        }),
      );

    const select = (name: string) => {
      setSelected(name);
      onSelect(name);
    };

    const menuEntry = entries.find((e) => e.slicer.name === menu?.name);

    return (
      <>
        <div className="ic-slicer-layer">
          {entries.map((entry) => {
            const { slicer } = entry;
            const box = boxOf(entry);
            const isSelected = selected === slicer.name;
            const { accent, solid } = slicerColors(model, slicer.style);
            const style = {
              left: box.left,
              top: box.top,
              width: box.width,
              height: box.height,
              "--ic-slicer-accent": accent,
            } as CSSProperties;
            return (
              <section
                key={slicer.name}
                className={`ic-slicer${isSelected ? " ic-slicer--selected" : ""}${
                  solid ? " ic-slicer--solid" : ""
                }`}
                style={style}
                aria-label={slicer.caption}
                tabIndex={-1}
                onPointerDown={(event) => {
                  event.stopPropagation();
                  select(slicer.name);
                }}
                onDoubleClick={(event) => event.stopPropagation()}
                onClick={(event) => event.stopPropagation()}
                onContextMenu={(event) => {
                  event.preventDefault();
                  event.stopPropagation();
                  if (canEdit) {
                    select(slicer.name);
                    setMenu({
                      name: slicer.name,
                      x: event.clientX,
                      y: event.clientY,
                    });
                  }
                }}
                onKeyDown={(event) => {
                  event.stopPropagation();
                  if (canEdit && event.key === "Delete" && isSelected) {
                    run(() => model.deleteSlicer(slicer.name));
                  }
                }}
              >
                {slicer.show_header ? (
                  <div
                    className="ic-slicer-header"
                    onPointerDown={(event) => {
                      if (
                        !(event.target as HTMLElement).closest(
                          ".ic-slicer-header-button",
                        )
                      ) {
                        beginDrag(event, entry, "move");
                      }
                    }}
                  >
                    <span className="ic-slicer-caption">{slicer.caption}</span>
                    <button
                      type="button"
                      className={`ic-slicer-header-button${
                        slicer.multi_select
                          ? " ic-slicer-header-button--pressed"
                          : ""
                      }`}
                      aria-label={t("table.multi_select")}
                      aria-pressed={slicer.multi_select}
                      title={t("table.multi_select")}
                      disabled={!canEdit}
                      onClick={() => setMultiSelect(entry)}
                    >
                      <ListChecks />
                    </button>
                    <button
                      type="button"
                      className="ic-slicer-header-button"
                      aria-label={t("table.clear_filter")}
                      title={t("table.clear_filter")}
                      disabled={!canEdit || !entry.filtered}
                      onClick={() =>
                        run(() => model.setSlicerSelection(slicer.name, []))
                      }
                    >
                      <FilterX />
                    </button>
                  </div>
                ) : null}
                <div
                  className="ic-slicer-items"
                  style={{
                    gridTemplateColumns: `repeat(${Math.max(slicer.button_columns, 1)}, minmax(0, 1fr))`,
                    gridAutoRows: slicer.button_height,
                  }}
                >
                  {entry.items.map((item) => (
                    <button
                      type="button"
                      key={item.label}
                      className={`ic-slicer-item${
                        item.visible ? " ic-slicer-item--on" : ""
                      }${item.has_data ? "" : " ic-slicer-item--empty"}`}
                      aria-pressed={item.visible}
                      title={item.label}
                      disabled={!canEdit}
                      onClick={(event) =>
                        toggleItem(entry, item, event.ctrlKey || event.metaKey)
                      }
                    >
                      {item.label}
                    </button>
                  ))}
                </div>
                {isSelected && canEdit && !slicer.locked
                  ? HANDLES.map((handle) => (
                      <div
                        key={handle}
                        className={`ic-slicer-handle ic-slicer-handle--${handle}`}
                        onPointerDown={(event) => {
                          event.stopPropagation();
                          beginDrag(event, entry, handle);
                        }}
                      />
                    ))
                  : null}
              </section>
            );
          })}
        </div>
        {/* biome-ignore lint/a11y/noStaticElementInteractions: presses in the menu must not reach the sheet behind it */}
        <div
          onPointerDown={(event) => event.stopPropagation()}
          onContextMenu={(event) => event.stopPropagation()}
        >
          {menu && menuEntry ? (
            <Menu
              open
              onClose={() => setMenu(null)}
              anchorPosition={{ x: menu.x, y: menu.y }}
            >
              <MenuItem
                icon={<FilterX />}
                disabled={!menuEntry.filtered}
                onClick={() =>
                  run(() => model.setSlicerSelection(menu.name, []))
                }
              >
                {t("table.clear_filter")}
              </MenuItem>
              <MenuItem
                checked={menuEntry.slicer.multi_select}
                onClick={() => setMultiSelect(menuEntry)}
              >
                {t("table.multi_select")}
              </MenuItem>
              <MenuItem
                icon={<Trash2 />}
                onClick={() => run(() => model.deleteSlicer(menu.name))}
              >
                {t("table.delete")}
              </MenuItem>
              <MenuDivider />
              <MenuItem
                icon={<Settings2 />}
                onClick={() => onShowSettings(menu.name)}
              >
                {t("table.show_settings")}
              </MenuItem>
            </Menu>
          ) : null}
          {alert}
        </div>
      </>
    );
  },
);
