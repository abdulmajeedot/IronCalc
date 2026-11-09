import {
  Clipboard,
  ClipboardPlus,
  Copy,
  Filter,
  Plus,
  Scissors,
  StickyNote,
  Trash2,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { Menu } from "../../Menu/Menu";
import { MenuDivider } from "../../Menu/MenuDivider";
import { MenuItem, MenuItemWithSubmenu } from "../../Menu/MenuItem";

interface CellContextMenuProps {
  open: boolean;
  onClose: () => void;
  anchorPosition: { x: number; y: number } | null;
  column: string;
  row: number;
  onInsertColumnLeft: () => void;
  onInsertColumnRight: () => void;
  onInsertRowAbove: () => void;
  onInsertRowBelow: () => void;
  onDeleteColumn: () => void;
  onDeleteRow: () => void;
  onCut: () => void;
  onCopy: () => void;
  onPaste: () => void;
  // Shown once something has been copied or cut
  insertCopiedLabel: string | null;
  onInsertCopied: () => void;
  rowCount: number;
  columnCount: number;
  onShiftCellsRight: () => void;
  onShiftCellsDown: () => void;
  hasNote: boolean;
  onEditNote: () => void;
  onDeleteNote: () => void;
  // Shown when the cell is under a sheet's filter
  filterActions?: {
    value: string;
    filtered: boolean;
    onFilterByValue: () => void;
    onClear: () => void;
    onReapply: () => void;
  };
}

const CellContextMenu = (properties: CellContextMenuProps) => {
  const { t } = useTranslation();
  const {
    open,
    onClose,
    anchorPosition,
    column,
    row,
    onInsertColumnLeft,
    onInsertColumnRight,
    onInsertRowAbove,
    onInsertRowBelow,
    onDeleteColumn,
    onDeleteRow,
    onCut,
    onCopy,
    onPaste,
    insertCopiedLabel,
    onInsertCopied,
    rowCount,
    columnCount,
    onShiftCellsRight,
    onShiftCellsDown,
    hasNote,
    onEditNote,
    onDeleteNote,
    filterActions,
  } = properties;

  if (!anchorPosition) {
    return null;
  }

  return (
    <Menu open={open} onClose={onClose} anchorPosition={anchorPosition}>
      <MenuItem icon={<Scissors />} onClick={onCut}>
        {t("context_menu.cell.cut")}
      </MenuItem>
      <MenuItem icon={<Copy />} onClick={onCopy}>
        {t("context_menu.cell.copy")}
      </MenuItem>
      <MenuItem icon={<Clipboard />} onClick={onPaste}>
        {t("context_menu.cell.paste")}
      </MenuItem>

      <MenuDivider />

      <MenuItemWithSubmenu
        icon={<Plus />}
        submenu={
          <>
            {insertCopiedLabel ? (
              <>
                <MenuItem icon={<ClipboardPlus />} onClick={onInsertCopied}>
                  {insertCopiedLabel}
                </MenuItem>
                <MenuDivider />
              </>
            ) : null}
            <MenuItem onClick={onInsertRowAbove}>
              {rowCount === 1
                ? t("context_menu.cell.insert_row_above")
                : t("context_menu.cell.insert_rows_above_count", {
                    count: rowCount,
                  })}
            </MenuItem>
            <MenuItem onClick={onInsertRowBelow}>
              {rowCount === 1
                ? t("context_menu.cell.insert_row_below")
                : t("context_menu.cell.insert_rows_below_count", {
                    count: rowCount,
                  })}
            </MenuItem>
            <MenuDivider />
            <MenuItem onClick={onInsertColumnLeft}>
              {columnCount === 1
                ? t("context_menu.cell.insert_column_left")
                : t("context_menu.cell.insert_columns_left_count", {
                    count: columnCount,
                  })}
            </MenuItem>
            <MenuItem onClick={onInsertColumnRight}>
              {columnCount === 1
                ? t("context_menu.cell.insert_column_right")
                : t("context_menu.cell.insert_columns_right_count", {
                    count: columnCount,
                  })}
            </MenuItem>
            <MenuDivider />
            <MenuItem onClick={onShiftCellsRight}>
              {t("context_menu.cell.shift_cells_right")}
            </MenuItem>
            <MenuItem onClick={onShiftCellsDown}>
              {t("context_menu.cell.shift_cells_down")}
            </MenuItem>
          </>
        }
      >
        {t("context_menu.cell.insert")}
      </MenuItemWithSubmenu>

      <MenuItemWithSubmenu
        icon={<Trash2 />}
        submenu={
          <>
            <MenuItem onClick={onDeleteColumn}>
              {t("context_menu.cell.delete_column", { column })}
            </MenuItem>
            <MenuItem onClick={onDeleteRow}>
              {t("context_menu.cell.delete_row", { row })}
            </MenuItem>
          </>
        }
      >
        {t("context_menu.cell.delete")}
      </MenuItemWithSubmenu>

      <MenuDivider />

      {filterActions ? (
        <MenuItemWithSubmenu
          icon={<Filter />}
          submenu={
            <>
              <MenuItem onClick={filterActions.onFilterByValue}>
                {t("table.filter_cell_value", { value: filterActions.value })}
              </MenuItem>
              <MenuItem
                disabled={!filterActions.filtered}
                onClick={filterActions.onClear}
              >
                {t("table.clear_all_filters")}
              </MenuItem>
              <MenuItem
                disabled={!filterActions.filtered}
                onClick={filterActions.onReapply}
              >
                {t("toolbar.reapply_filters")}
              </MenuItem>
            </>
          }
        >
          {t("table.filter")}
        </MenuItemWithSubmenu>
      ) : null}

      <MenuItem icon={<StickyNote />} onClick={onEditNote}>
        {t(
          hasNote
            ? "context_menu.cell.edit_note"
            : "context_menu.cell.new_note",
        )}
      </MenuItem>
      {hasNote ? (
        <MenuItem onClick={onDeleteNote}>
          {t("context_menu.cell.delete_note")}
        </MenuItem>
      ) : null}
    </Menu>
  );
};

export default CellContextMenu;
