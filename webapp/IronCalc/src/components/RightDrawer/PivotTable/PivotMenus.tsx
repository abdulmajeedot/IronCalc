import type {
  Model,
  PivotAggregation,
  PivotCellInfo,
  PivotField,
  PivotLabelCondition,
  PivotMarker,
  PivotShowAs,
  PivotTable,
  PivotTableEntry,
  PivotValueCondition,
  PivotValueRule,
} from "@ironcalc/wasm";
import {
  ArrowDownAZ,
  ArrowDownZA,
  Copy,
  FilterX,
  ListTree,
  RefreshCw,
  Settings2,
  Trash2,
  X,
} from "lucide-react";
import {
  type KeyboardEvent,
  type ReactNode,
  useEffect,
  useId,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../Button/Button";
import { IconButton } from "../../Button/IconButton";
import { Checkbox } from "../../Checkbox/Checkbox";
import { Input } from "../../Input/Input";
import { Menu } from "../../Menu/Menu";
import { MenuDivider } from "../../Menu/MenuDivider";
import { MenuItem, MenuItemWithSubmenu } from "../../Menu/MenuItem";
import { Alert, Confirm } from "../../Modal";
import { ModalDialog } from "../../Modal/ModalDialog";
import { useModalFocus } from "../../Modal/useModalFocus";
import { useModalKeyDown } from "../../Modal/useModalKeyDown";
import {
  AGGREGATIONS,
  displayOrder,
  type ItemMove,
  moveFieldInZone,
  moveItem,
  patchField,
  patchValue,
  removeAt,
  removeField,
  setAllCollapsed,
  toggleCollapsed,
  zoneOfField,
} from "./pivotLogic";
import "./pivot-table.css";

const OVERWRITE_ERROR = "PIVOT_OVERWRITE";

const LABEL_GROUPS: PivotLabelCondition[][] = [
  ["equals", "not_equals"],
  ["begins_with", "not_begins_with", "ends_with", "not_ends_with"],
  ["contains", "not_contains"],
  ["greater_than", "greater_than_or_equal", "less_than", "less_than_or_equal"],
  ["between", "not_between"],
];

const VALUE_GROUPS: PivotValueCondition[][] = [
  ["equals", "not_equals"],
  ["greater_than", "greater_than_or_equal", "less_than", "less_than_or_equal"],
  ["between", "not_between"],
];

const SHOW_AS: PivotShowAs[] = [
  "normal",
  "percent_of_total",
  "percent_of_column",
  "percent_of_row",
  "percent_of_parent_row",
  "percent_of_parent_column",
];

type Dialog =
  | { kind: "label"; field: string; condition: PivotLabelCondition }
  | { kind: "value"; field: string; condition: PivotValueCondition }
  | { kind: "top"; field: string }
  | { kind: "items"; field: string }
  | { kind: "sort"; field: string };

export type PivotMenuState =
  | { type: "cell"; info: PivotCellInfo; x: number; y: number }
  | { type: "dropdown"; marker: PivotMarker; x: number; y: number };

interface PivotMenusProps {
  model: Model;
  state: PivotMenuState | null;
  onClose: () => void;
  refresh: () => void;
  onCopy: () => void;
  fieldListOpen: boolean;
  onToggleFieldList: () => void;
  onShowSettings: () => void;
}

const messageOf = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

const between = (condition: string) =>
  condition === "between" || condition === "not_between";

function needsNoValue(values: PivotTable["values"]): boolean {
  return values.length === 0;
}

const isFiltered = (field: PivotField | undefined): boolean =>
  !!field &&
  (field.hidden_items.length > 0 ||
    !!field.label_filter ||
    !!field.value_filter);

interface ChecklistProps {
  items: string[];
  hidden: string[];
  onApply: (hidden: string[]) => void;
  applyLabel: string;
  searchLabel: string;
  selectAllLabel: string;
}

// The searchable list of items with Select All.
export const Checklist = ({
  items,
  hidden: initial,
  onApply,
  applyLabel,
  searchLabel,
  selectAllLabel,
}: ChecklistProps) => {
  const [hidden, setHidden] = useState(() => new Set(initial));
  const [search, setSearch] = useState("");
  const query = search.trim().toLowerCase();
  const shown = items.filter((i) => i.toLowerCase().includes(query));
  const allShownVisible = shown.every((i) => !hidden.has(i));
  const nothingVisible = items.every((i) => hidden.has(i));
  const toggle = (item: string, visible: boolean) => {
    const next = new Set(hidden);
    if (visible) {
      next.delete(item);
    } else {
      next.add(item);
    }
    setHidden(next);
  };
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: keeps typing here away from the sheet
    <div
      className="ic-pivot-checklist"
      onKeyDown={(event) => {
        if (event.key !== "Escape" && event.key !== "Tab") {
          event.stopPropagation();
        }
      }}
    >
      <Input
        type="text"
        size="sm"
        value={search}
        placeholder={searchLabel}
        aria-label={searchLabel}
        onChange={(event) => setSearch(event.target.value)}
      />
      <div className="ic-pivot-checklist-items">
        <Checkbox
          label={selectAllLabel}
          checked={allShownVisible}
          onChange={(visible) => {
            const next = new Set(hidden);
            for (const item of shown) {
              if (visible) {
                next.delete(item);
              } else {
                next.add(item);
              }
            }
            setHidden(next);
          }}
        />
        {shown.map((item) => (
          <Checkbox
            key={item}
            label={item}
            checked={!hidden.has(item)}
            onChange={(visible) => toggle(item, visible)}
          />
        ))}
      </div>
      <Button
        size="sm"
        disabled={nothingVisible}
        onClick={() => onApply(items.filter((i) => hidden.has(i)))}
      >
        {applyLabel}
      </Button>
    </div>
  );
};

interface DialogShellProps {
  title: string;
  onClose: () => void;
  onConfirm?: () => void;
  confirmDisabled?: boolean;
  children: React.ReactNode;
  confirmLabel?: string;
  // Shown in the footer before Cancel
  footerStart?: ReactNode;
}

export const DialogShell = ({
  title,
  onClose,
  onConfirm,
  confirmDisabled,
  confirmLabel,
  footerStart,
  children,
}: DialogShellProps) => {
  const { t } = useTranslation();
  const titleId = useId();
  const { modalRef, restoreFocus } = useModalFocus(true);
  const closeRef = useRef<HTMLButtonElement>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);
  const close = () => {
    onClose();
    restoreFocus();
  };
  const confirm = () => {
    onConfirm?.();
    restoreFocus();
  };
  const { onKeyDown } = useModalKeyDown({
    focusableElements: [closeRef, confirmRef],
    onClose: close,
    onConfirm: confirm,
  });
  const stopFromSheet = (event: KeyboardEvent<HTMLDivElement>) => {
    event.stopPropagation();
    if (event.key === "Escape") {
      event.preventDefault();
      close();
    } else if (event.key === "Enter" && onConfirm && !confirmDisabled) {
      const target = event.target as HTMLElement;
      if (target.tagName === "INPUT") {
        event.preventDefault();
        confirm();
      }
    }
  };
  return (
    <ModalDialog
      modalRef={modalRef}
      titleId={titleId}
      onClose={close}
      onKeyDown={onKeyDown}
    >
      <div className="ic-modal-dialog-header">
        <h2 id={titleId}>{title}</h2>
        <IconButton
          ref={closeRef}
          icon={<X />}
          aria-label={t("right_drawer.close")}
          onClick={close}
        />
      </div>
      {/* biome-ignore lint/a11y/noStaticElementInteractions: keeps typing here away from the sheet */}
      <div className="ic-modal-dialog-body" onKeyDown={stopFromSheet}>
        {children}
      </div>
      {onConfirm ? (
        <div className="ic-modal-dialog-footer">
          {footerStart}
          <Button size="md" variant="secondary" onClick={close}>
            {t("common.cancel")}
          </Button>
          <Button
            ref={confirmRef}
            size="md"
            disabled={confirmDisabled}
            onClick={confirm}
          >
            {confirmLabel ?? t("pivot_table.ok")}
          </Button>
        </div>
      ) : null}
    </ModalDialog>
  );
};

interface PivotDialogProps {
  dialog: Dialog;
  pivot: PivotTable;
  items: string[];
  onClose: () => void;
  onApply: (next: PivotTable) => void;
}

const PivotDialog = ({
  dialog,
  pivot,
  items,
  onClose,
  onApply,
}: PivotDialogProps) => {
  const { t } = useTranslation();
  const { field } = dialog;
  const layout = [...pivot.rows, ...pivot.columns, ...pivot.filters].find(
    (f) => f.field === field,
  );
  const captions = pivot.values.map(
    (v) =>
      v.name ||
      `${t(`pivot_table.aggregations.${v.aggregation}`)} of ${v.field}`,
  );
  const engineCaption = (index: number) => {
    const value = pivot.values[index];
    return (
      value.name || `${ENGINE_CAPTIONS[value.aggregation]} of ${value.field}`
    );
  };
  const [text1, setText1] = useState(() => {
    if (dialog.kind === "label") {
      return layout?.label_filter?.value ?? "";
    }
    if (dialog.kind === "value") {
      const rule = layout?.value_filter?.rule;
      return rule?.type === "compare" ? String(rule.value) : "";
    }
    if (dialog.kind === "top") {
      const rule = layout?.value_filter?.rule;
      return rule?.type === "top" ? String(rule.count) : "10";
    }
    return "";
  });
  const [text2, setText2] = useState(() => {
    if (dialog.kind === "label") {
      return layout?.label_filter?.value2 ?? "";
    }
    const rule = layout?.value_filter?.rule;
    return rule?.type === "compare" ? String(rule.value2) : "";
  });
  const [valueIndex, setValueIndex] = useState(() => {
    const name = layout?.value_filter?.value_name ?? layout?.sort.by_value;
    const found = pivot.values.findIndex((_, i) => engineCaption(i) === name);
    return Math.max(found, 0);
  });
  const [bottom, setBottom] = useState(false);
  const [unit, setUnit] = useState<"items" | "percent" | "sum">("items");
  const [descending, setDescending] = useState(
    layout?.sort.order === "descending",
  );

  const valueSelect = (
    <>
      <div className="ic-pivot-label">{t("pivot_table.by_value")}</div>
      <select
        className="ic-pivot-select"
        aria-label={t("pivot_table.by_value")}
        value={valueIndex}
        onChange={(event) => setValueIndex(Number(event.target.value))}
      >
        {captions.map((caption, index) => (
          <option key={caption} value={index}>
            {caption}
          </option>
        ))}
      </select>
    </>
  );

  if (dialog.kind === "items") {
    return (
      <DialogShell
        title={t("pivot_table.filter_title", { field })}
        onClose={onClose}
      >
        <Checklist
          items={items}
          hidden={layout?.hidden_items ?? []}
          applyLabel={t("pivot_table.ok")}
          searchLabel={t("pivot_table.search_items")}
          selectAllLabel={t("pivot_table.select_all")}
          onApply={(hidden) =>
            onApply(patchField(pivot, field, { hidden_items: hidden }))
          }
        />
      </DialogShell>
    );
  }

  if (dialog.kind === "sort") {
    return (
      <DialogShell
        title={t("pivot_table.sort_by_value_title", { field })}
        onClose={onClose}
        onConfirm={() =>
          onApply(
            patchField(pivot, field, {
              sort: {
                order: descending ? "descending" : "ascending",
                by_value: engineCaption(valueIndex),
              },
            }),
          )
        }
      >
        {valueSelect}
        <div className="ic-pivot-label">{t("pivot_table.sort_options")}</div>
        <select
          className="ic-pivot-select"
          aria-label={t("pivot_table.sort_options")}
          value={descending ? "descending" : "ascending"}
          onChange={(event) =>
            setDescending(event.target.value === "descending")
          }
        >
          <option value="ascending">{t("pivot_table.smallest_first")}</option>
          <option value="descending">{t("pivot_table.largest_first")}</option>
        </select>
      </DialogShell>
    );
  }

  if (dialog.kind === "top") {
    const count = Number(text1);
    return (
      <DialogShell
        title={t("pivot_table.top10_title", { field })}
        onClose={onClose}
        confirmDisabled={!Number.isFinite(count) || count <= 0}
        onConfirm={() => {
          const rule: PivotValueRule = { type: "top", count, bottom, unit };
          onApply(
            patchField(pivot, field, {
              label_filter: null,
              value_filter: { value_name: engineCaption(valueIndex), rule },
            }),
          );
        }}
      >
        <div className="ic-pivot-row">
          <select
            className="ic-pivot-select"
            aria-label={t("pivot_table.top_or_bottom")}
            value={bottom ? "bottom" : "top"}
            onChange={(event) => setBottom(event.target.value === "bottom")}
          >
            <option value="top">{t("pivot_table.top")}</option>
            <option value="bottom">{t("pivot_table.bottom")}</option>
          </select>
          <Input
            autoFocus
            type="text"
            inputMode="decimal"
            size="sm"
            value={text1}
            aria-label={t("pivot_table.how_many")}
            onChange={(event) => setText1(event.target.value)}
          />
          <select
            className="ic-pivot-select"
            aria-label={t("pivot_table.top_unit")}
            value={unit}
            onChange={(event) => setUnit(event.target.value as typeof unit)}
          >
            <option value="items">{t("pivot_table.unit_items")}</option>
            <option value="percent">{t("pivot_table.unit_percent")}</option>
            <option value="sum">{t("pivot_table.unit_sum")}</option>
          </select>
        </div>
        {valueSelect}
      </DialogShell>
    );
  }

  const twoValues = between(dialog.condition);
  const inputs = (inputMode: "text" | "decimal") => (
    <div className="ic-pivot-row">
      <Input
        autoFocus
        type="text"
        inputMode={inputMode}
        size="sm"
        value={text1}
        aria-label={t("pivot_table.filter_value")}
        onChange={(event) => setText1(event.target.value)}
      />
      {twoValues ? (
        <>
          <span>{t("pivot_table.and")}</span>
          <Input
            type="text"
            inputMode={inputMode}
            size="sm"
            value={text2}
            aria-label={t("pivot_table.filter_value_2")}
            onChange={(event) => setText2(event.target.value)}
          />
        </>
      ) : null}
    </div>
  );

  if (dialog.kind === "label") {
    const condition = dialog.condition;
    return (
      <DialogShell
        title={t("pivot_table.label_filter_title", { field })}
        onClose={onClose}
        confirmDisabled={text1 === "" || (twoValues && text2 === "")}
        onConfirm={() =>
          onApply(
            patchField(pivot, field, {
              value_filter: null,
              label_filter: { condition, value: text1, value2: text2 },
            }),
          )
        }
      >
        <div className="ic-pivot-label">
          {t(`pivot_table.conditions.${condition}`)}
        </div>
        {inputs("text")}
      </DialogShell>
    );
  }

  const condition = dialog.condition;
  const first = Number(text1);
  const second = Number(text2);
  const invalid =
    text1 === "" ||
    !Number.isFinite(first) ||
    (twoValues && (text2 === "" || !Number.isFinite(second)));
  return (
    <DialogShell
      title={t("pivot_table.value_filter_title", { field })}
      onClose={onClose}
      confirmDisabled={invalid}
      onConfirm={() =>
        onApply(
          patchField(pivot, field, {
            label_filter: null,
            value_filter: {
              value_name: engineCaption(valueIndex),
              rule: {
                type: "compare",
                condition,
                value: first,
                value2: twoValues ? second : 0,
              },
            },
          }),
        )
      }
    >
      {valueSelect}
      <div className="ic-pivot-label">
        {t(`pivot_table.conditions.${condition}`)}
      </div>
      {inputs("decimal")}
    </DialogShell>
  );
};

// The captions the engine uses, which are not translated.
const ENGINE_CAPTIONS: Record<PivotAggregation, string> = {
  sum: "Sum",
  count: "Count",
  average: "Average",
  max: "Max",
  min: "Min",
  product: "Product",
  count_numbers: "Count Numbers",
  std_dev: "StdDev",
  std_dev_p: "StdDevp",
  var: "Var",
  var_p: "Varp",
};

export function PivotMenus({
  model,
  state,
  onClose,
  refresh,
  onCopy,
  fieldListOpen,
  onToggleFieldList,
  onShowSettings,
}: PivotMenusProps) {
  const { t } = useTranslation();
  const [dialog, setDialog] = useState<{
    dialog: Dialog;
    index: number;
  } | null>(null);
  const [pending, setPending] = useState<{
    index: number;
    next: PivotTable;
  } | null>(null);
  const [error, setError] = useState("");

  const index =
    state === null
      ? (dialog?.index ?? null)
      : state.type === "cell"
        ? state.info.index
        : state.marker.index;
  const entry: PivotTableEntry | undefined =
    index === null
      ? undefined
      : model.getPivotTables().find((e) => e.index === index);

  // Only read while a menu or dialog is open
  const fieldItems = new Map<string, string[]>();
  if (
    entry &&
    entry.source_sheet !== null &&
    entry.source_sheet !== undefined
  ) {
    try {
      for (const info of model.getPivotFields(
        entry.source_sheet,
        entry.pivot.source,
      )) {
        fieldItems.set(info.name, info.items);
      }
    } catch {
      // A pivot whose source is gone still offers its other commands
    }
  }

  useEffect(() => {
    if (dialog && !entry) {
      setDialog(null);
    }
  }, [dialog, entry]);

  const apply = (next: PivotTable, overwrite = false) => {
    if (!entry) {
      return;
    }
    try {
      model.updatePivotTable(entry.index, next, overwrite);
      refresh();
    } catch (e) {
      const message = messageOf(e);
      if (message === OVERWRITE_ERROR) {
        setPending({ index: entry.index, next });
      } else {
        setError(message);
      }
    }
  };

  const run = (action: () => void) => {
    try {
      action();
      refresh();
    } catch (e) {
      setError(messageOf(e));
    }
  };

  const overlays = (
    <>
      {dialog && entry ? (
        <PivotDialog
          dialog={dialog.dialog}
          pivot={entry.pivot}
          items={fieldItems.get(dialog.dialog.field) ?? []}
          onClose={() => setDialog(null)}
          onApply={(next) => {
            setDialog(null);
            apply(next);
          }}
        />
      ) : null}
      <Confirm
        open={pending !== null}
        title={t("pivot_table.replace")}
        message={t("pivot_table.replace_warning")}
        confirmLabel={t("pivot_table.replace")}
        onClose={() => setPending(null)}
        onConfirm={() => {
          if (pending) {
            try {
              model.updatePivotTable(pending.index, pending.next, true);
              refresh();
            } catch (e) {
              setError(messageOf(e));
            }
          }
          setPending(null);
        }}
      />
      <Alert
        open={error !== ""}
        title={t("pivot_table.error_title")}
        message={error}
        onClose={() => setError("")}
      />
    </>
  );

  if (!state || !entry) {
    return overlays;
  }

  const { pivot } = entry;
  const openDialog = (next: Dialog) =>
    setDialog({ dialog: next, index: entry.index });
  const hasNoValues = needsNoValue(pivot.values);

  const sortItems = (field: string) => {
    const zone = zoneOfField(pivot, field);
    const sortable = zone === "rows" || zone === "columns";
    return (
      <>
        <MenuItem
          icon={<ArrowDownAZ />}
          disabled={!sortable}
          onClick={() =>
            apply(
              patchField(pivot, field, {
                sort: { order: "ascending", by_value: null },
              }),
            )
          }
        >
          {t("pivot_table.sort_ascending")}
        </MenuItem>
        <MenuItem
          icon={<ArrowDownZA />}
          disabled={!sortable}
          onClick={() =>
            apply(
              patchField(pivot, field, {
                sort: { order: "descending", by_value: null },
              }),
            )
          }
        >
          {t("pivot_table.sort_descending")}
        </MenuItem>
        <MenuItem
          disabled={!sortable || hasNoValues}
          onClick={() => openDialog({ kind: "sort", field })}
        >
          {t("pivot_table.sort_by_value")}
        </MenuItem>
      </>
    );
  };

  const filterEntries = (field: string) => {
    const layout = [...pivot.rows, ...pivot.columns, ...pivot.filters].find(
      (f) => f.field === field,
    );
    const zone = zoneOfField(pivot, field);
    const onAxis = zone === "rows" || zone === "columns";
    return (
      <>
        <MenuItem
          icon={<FilterX />}
          disabled={!isFiltered(layout)}
          onClick={() =>
            apply(
              patchField(pivot, field, {
                hidden_items: [],
                label_filter: null,
                value_filter: null,
              }),
            )
          }
        >
          {t("pivot_table.clear_filter", { field })}
        </MenuItem>
        {onAxis ? (
          <>
            <MenuItemWithSubmenu
              submenu={
                <>
                  <MenuItem
                    disabled={!layout?.label_filter}
                    onClick={() =>
                      apply(patchField(pivot, field, { label_filter: null }))
                    }
                  >
                    {t("pivot_table.clear_this_filter")}
                  </MenuItem>
                  {LABEL_GROUPS.map((group) => (
                    <div key={group[0]}>
                      <MenuDivider />
                      {group.map((condition) => (
                        <MenuItem
                          key={condition}
                          onClick={() =>
                            openDialog({ kind: "label", field, condition })
                          }
                        >
                          {t(`pivot_table.conditions.${condition}`)}
                        </MenuItem>
                      ))}
                    </div>
                  ))}
                </>
              }
            >
              {t("pivot_table.label_filters")}
            </MenuItemWithSubmenu>
            <MenuItemWithSubmenu
              submenu={
                <>
                  <MenuItem
                    disabled={!layout?.value_filter}
                    onClick={() =>
                      apply(patchField(pivot, field, { value_filter: null }))
                    }
                  >
                    {t("pivot_table.clear_this_filter")}
                  </MenuItem>
                  {VALUE_GROUPS.map((group) => (
                    <div key={group[0]}>
                      <MenuDivider />
                      {group.map((condition) => (
                        <MenuItem
                          key={condition}
                          disabled={hasNoValues}
                          onClick={() =>
                            openDialog({ kind: "value", field, condition })
                          }
                        >
                          {t(`pivot_table.conditions.${condition}`)}
                        </MenuItem>
                      ))}
                    </div>
                  ))}
                  <MenuDivider />
                  <MenuItem
                    disabled={hasNoValues}
                    onClick={() => openDialog({ kind: "top", field })}
                  >
                    {t("pivot_table.top_10")}
                  </MenuItem>
                </>
              }
            >
              {t("pivot_table.value_filters")}
            </MenuItemWithSubmenu>
          </>
        ) : null}
      </>
    );
  };

  const anchor = { x: state.x, y: state.y };
  const fieldListLabel = fieldListOpen
    ? t("pivot_table.hide_field_list")
    : t("pivot_table.show_field_list");

  if (state.type === "dropdown") {
    const { field } = state.marker;
    const layout = [...pivot.rows, ...pivot.columns, ...pivot.filters].find(
      (f) => f.field === field,
    );
    return (
      <>
        {overlays}
        <Menu open onClose={onClose} anchorPosition={anchor}>
          {sortItems(field)}
          <MenuDivider />
          {filterEntries(field)}
          <MenuDivider />
          <Checklist
            key={field}
            items={fieldItems.get(field) ?? []}
            hidden={layout?.hidden_items ?? []}
            applyLabel={t("pivot_table.apply")}
            searchLabel={t("pivot_table.search_items")}
            selectAllLabel={t("pivot_table.select_all")}
            onApply={(hidden) => {
              onClose();
              apply(patchField(pivot, field, { hidden_items: hidden }));
            }}
          />
        </Menu>
      </>
    );
  }

  const { info } = state;
  const common = (
    <>
      <MenuItem icon={<Copy />} onClick={onCopy}>
        {t("pivot_table.copy")}
      </MenuItem>
      <MenuDivider />
      <MenuItem
        icon={<RefreshCw />}
        disabled={pivot.read_only}
        onClick={() => run(() => model.refreshPivotTable(entry.index))}
      >
        {t("pivot_table.refresh_short")}
      </MenuItem>
      <MenuDivider />
      <MenuItem
        icon={<Trash2 />}
        onClick={() => run(() => model.deletePivotTable(entry.index))}
      >
        {t("pivot_table.delete")}
      </MenuItem>
      <MenuDivider />
    </>
  );
  const footer = (
    <>
      <MenuDivider />
      <MenuItem icon={<Settings2 />} onClick={onShowSettings}>
        {t("pivot_table.show_settings")}
      </MenuItem>
      <MenuItem icon={<ListTree />} onClick={onToggleFieldList}>
        {fieldListLabel}
      </MenuItem>
    </>
  );

  if (pivot.read_only || info.zone === "total") {
    return (
      <>
        {overlays}
        <Menu open onClose={onClose} anchorPosition={anchor}>
          {common}
          {footer}
        </Menu>
      </>
    );
  }

  if (info.zone === "value") {
    const position = info.value_index ?? 0;
    const value = pivot.values[position];
    const sortField =
      pivot.rows[pivot.rows.length - 1]?.field ??
      pivot.columns[pivot.columns.length - 1]?.field;
    const caption =
      value?.name ||
      (value
        ? `${ENGINE_CAPTIONS[value.aggregation]} of ${value.field}`
        : undefined);
    const sortByThis = (order: "ascending" | "descending") => {
      if (sortField && caption) {
        apply(
          patchField(pivot, sortField, { sort: { order, by_value: caption } }),
        );
      }
    };
    return (
      <>
        {overlays}
        <Menu open onClose={onClose} anchorPosition={anchor}>
          {common}
          <MenuItemWithSubmenu
            submenu={
              <>
                <MenuItem
                  icon={<ArrowDownAZ />}
                  disabled={!sortField || !value}
                  onClick={() => sortByThis("ascending")}
                >
                  {t("pivot_table.sort_smallest")}
                </MenuItem>
                <MenuItem
                  icon={<ArrowDownZA />}
                  disabled={!sortField || !value}
                  onClick={() => sortByThis("descending")}
                >
                  {t("pivot_table.sort_largest")}
                </MenuItem>
              </>
            }
          >
            {t("pivot_table.sort")}
          </MenuItemWithSubmenu>
          <MenuDivider />
          <MenuItem
            icon={<Trash2 />}
            disabled={!value}
            onClick={() => apply(removeAt(pivot, "values", position))}
          >
            {t("pivot_table.remove_named", { field: caption ?? "" })}
          </MenuItem>
          <MenuDivider />
          <MenuItemWithSubmenu
            submenu={AGGREGATIONS.map((aggregation) => (
              <MenuItem
                key={aggregation}
                checked={value?.aggregation === aggregation}
                onClick={() =>
                  apply(patchValue(pivot, position, { aggregation }))
                }
              >
                {t(`pivot_table.aggregations.${aggregation}`)}
              </MenuItem>
            ))}
          >
            {t("pivot_table.summarize_values_by")}
          </MenuItemWithSubmenu>
          <MenuItemWithSubmenu
            submenu={
              <>
                {SHOW_AS.map((showAs) => (
                  <MenuItem
                    key={showAs}
                    checked={value?.show_as === showAs}
                    onClick={() =>
                      apply(
                        patchValue(pivot, position, {
                          show_as: showAs,
                          base_field: "",
                        }),
                      )
                    }
                  >
                    {t(`pivot_table.show_as.${showAs}`)}
                  </MenuItem>
                ))}
                <MenuItemWithSubmenu
                  submenu={[...pivot.rows, ...pivot.columns].map((axis) => (
                    <MenuItem
                      key={axis.field}
                      checked={
                        value?.show_as === "percent_of_parent" &&
                        value.base_field === axis.field
                      }
                      onClick={() =>
                        apply(
                          patchValue(pivot, position, {
                            show_as: "percent_of_parent",
                            base_field: axis.field,
                          }),
                        )
                      }
                    >
                      {axis.field}
                    </MenuItem>
                  ))}
                >
                  {t("pivot_table.show_as.percent_of_parent")}
                </MenuItemWithSubmenu>
              </>
            }
          >
            {t("pivot_table.show_values_as")}
          </MenuItemWithSubmenu>
          <MenuDivider />
          <MenuItem
            onClick={() =>
              run(() => {
                const view = model.getSelectedView();
                const names = model
                  .getWorksheetsProperties()
                  .map((w) => w.name.toLowerCase());
                let number = 1;
                while (names.includes(`details${number}`)) {
                  number += 1;
                }
                model.showPivotDetails(
                  view.sheet,
                  view.row,
                  view.column,
                  `Details${number}`,
                );
              })
            }
          >
            {t("pivot_table.show_details")}
          </MenuItem>
          {footer}
        </Menu>
      </>
    );
  }

  const field = info.field;
  if (!field) {
    return overlays;
  }
  const zone = zoneOfField(pivot, field);
  const layout = zone ? pivot[zone].find((f) => f.field === field) : undefined;
  const onAxis = zone === "rows" || zone === "columns";
  const subtotal = layout?.subtotal ?? pivot.options.subtotals;
  const compact = pivot.options.layout === "compact";
  const repeat = layout?.repeat_labels ?? pivot.options.repeat_labels;
  const order = displayOrder(pivot, field, fieldItems.get(field) ?? []);
  const itemIndex = info.item ? order.indexOf(info.item) : -1;
  const list = zone ? pivot[zone] : [];
  const fieldIndex = list.findIndex((f) => f.field === field);
  const shift = (where: ItemMove) => {
    if (info.item) {
      apply(moveItem(pivot, field, info.item, where, order));
    }
  };
  const shiftField = (delta: number | "first" | "last") => {
    if (zone) {
      apply(moveFieldInZone(pivot, zone, fieldIndex, delta));
    }
  };
  const itemName = info.item ?? "";
  const moveLabels = zone === "rows";
  return (
    <>
      {overlays}
      <Menu open onClose={onClose} anchorPosition={anchor}>
        {common}
        <MenuItemWithSubmenu submenu={sortItems(field)}>
          {t("pivot_table.sort")}
        </MenuItemWithSubmenu>
        <MenuItemWithSubmenu
          submenu={
            <>
              {filterEntries(field)}
              <MenuDivider />
              <MenuItem onClick={() => openDialog({ kind: "items", field })}>
                {t("pivot_table.filter_items")}
              </MenuItem>
            </>
          }
        >
          {t("pivot_table.filter")}
        </MenuItemWithSubmenu>
        <MenuDivider />
        {onAxis ? (
          <>
            <MenuItemWithSubmenu
              submenu={
                <>
                  {(["none", "top", "bottom"] as const).map((option) => (
                    <MenuItem
                      key={option}
                      checked={subtotal === option}
                      onClick={() =>
                        apply(patchField(pivot, field, { subtotal: option }))
                      }
                    >
                      {t(`pivot_table.subtotal_${option}`)}
                    </MenuItem>
                  ))}
                </>
              }
            >
              {t("pivot_table.subtotal_named", { field })}
            </MenuItemWithSubmenu>
            <MenuItem
              checked={repeat}
              disabled={compact}
              onClick={() =>
                apply(patchField(pivot, field, { repeat_labels: !repeat }))
              }
            >
              {t("pivot_table.repeat_labels")}
            </MenuItem>
            <MenuItem
              checked={layout?.blank_line ?? false}
              onClick={() =>
                apply(
                  patchField(pivot, field, {
                    blank_line: !(layout?.blank_line ?? false),
                  }),
                )
              }
            >
              {t("pivot_table.blank_line")}
            </MenuItem>
            <MenuDivider />
          </>
        ) : null}
        <MenuItemWithSubmenu
          submenu={
            <>
              <MenuItem
                disabled={itemIndex <= 0}
                onClick={() => shift("first")}
              >
                {t("pivot_table.move_item_first", { item: itemName })}
              </MenuItem>
              <MenuItem disabled={itemIndex <= 0} onClick={() => shift("up")}>
                {t("pivot_table.move_item_up", { item: itemName })}
              </MenuItem>
              <MenuItem
                disabled={itemIndex < 0 || itemIndex >= order.length - 1}
                onClick={() => shift("down")}
              >
                {t("pivot_table.move_item_down", { item: itemName })}
              </MenuItem>
              <MenuItem
                disabled={itemIndex < 0 || itemIndex >= order.length - 1}
                onClick={() => shift("last")}
              >
                {t("pivot_table.move_item_last", { item: itemName })}
              </MenuItem>
              <MenuDivider />
              <MenuItem
                disabled={fieldIndex <= 0}
                onClick={() => shiftField("first")}
              >
                {t("pivot_table.move_field_first", { field })}
              </MenuItem>
              <MenuItem
                disabled={fieldIndex <= 0}
                onClick={() => shiftField(-1)}
              >
                {t(
                  moveLabels
                    ? "pivot_table.move_field_left"
                    : "pivot_table.move_field_earlier",
                  { field },
                )}
              </MenuItem>
              <MenuItem
                disabled={fieldIndex < 0 || fieldIndex >= list.length - 1}
                onClick={() => shiftField(1)}
              >
                {t(
                  moveLabels
                    ? "pivot_table.move_field_right"
                    : "pivot_table.move_field_later",
                  { field },
                )}
              </MenuItem>
              <MenuItem
                disabled={fieldIndex < 0 || fieldIndex >= list.length - 1}
                onClick={() => shiftField("last")}
              >
                {t("pivot_table.move_field_last", { field })}
              </MenuItem>
            </>
          }
        >
          {t("pivot_table.move")}
        </MenuItemWithSubmenu>
        <MenuItem
          icon={<Trash2 />}
          onClick={() => apply(removeField(pivot, field))}
        >
          {t("pivot_table.remove_named", { field })}
        </MenuItem>
        {info.has_children ? (
          <>
            <MenuDivider />
            <MenuItemWithSubmenu
              submenu={
                <>
                  <MenuItem
                    disabled={!info.collapsed}
                    onClick={() =>
                      info.item &&
                      apply(toggleCollapsed(pivot, field, info.item))
                    }
                  >
                    {t("pivot_table.expand")}
                  </MenuItem>
                  <MenuItem
                    disabled={info.collapsed}
                    onClick={() =>
                      info.item &&
                      apply(toggleCollapsed(pivot, field, info.item))
                    }
                  >
                    {t("pivot_table.collapse")}
                  </MenuItem>
                  <MenuItem
                    onClick={() =>
                      apply(setAllCollapsed(pivot, field, order, false))
                    }
                  >
                    {t("pivot_table.expand_all")}
                  </MenuItem>
                  <MenuItem
                    onClick={() =>
                      apply(setAllCollapsed(pivot, field, order, true))
                    }
                  >
                    {t("pivot_table.collapse_all")}
                  </MenuItem>
                </>
              }
            >
              {t("pivot_table.expand_collapse")}
            </MenuItemWithSubmenu>
          </>
        ) : null}
        {footer}
      </Menu>
    </>
  );
}
