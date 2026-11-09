import type {
  Model,
  TableEntry,
  TableFilterOperator,
  TableMarker,
} from "@ironcalc/wasm";
import {
  ArrowDownAZ,
  ArrowDownZA,
  Clipboard,
  Copy,
  FilterX,
  Plus,
  Scissors,
  Table2,
  Trash2,
} from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Checkbox } from "../../Checkbox/Checkbox";
import { parseRangeInSheet } from "../../Editor/util";
import { Input } from "../../Input/Input";
import { Menu } from "../../Menu/Menu";
import { MenuDivider } from "../../Menu/MenuDivider";
import { MenuItem, MenuItemWithSubmenu } from "../../Menu/MenuItem";
import { Alert } from "../../Modal";
import { Select } from "../../Select/Select";
import { Checklist, DialogShell } from "../PivotTable/PivotMenus";
import { suggestSource } from "../PivotTable/PivotTablePanel";
import "./table.css";

export const BLANK_LABEL = "(Blanks)";

export const messageOf = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

export type TableMenuState =
  | { type: "dropdown"; marker: TableMarker; x: number; y: number }
  | { type: "cell"; table: string; x: number; y: number };

export type TableDialogState =
  | { kind: "create" }
  | { kind: "slicers"; table: string | null; pivot: number | null }
  | { kind: "resize"; table: string }
  | { kind: "duplicates"; table: string }
  | {
      kind: "filter";
      table: string;
      column: string;
      operator: TableFilterOperator;
    }
  | { kind: "top"; table: string; column: string };

const TEXT_OPERATORS: TableFilterOperator[] = [
  "equals",
  "not_equals",
  "begins_with",
  "ends_with",
  "contains",
  "not_contains",
];

const NUMBER_OPERATORS: TableFilterOperator[] = [
  "equals",
  "not_equals",
  "greater_than",
  "greater_than_or_equal",
  "less_than",
  "less_than_or_equal",
  "between",
];

interface TableMenusProps {
  model: Model;
  state: TableMenuState | null;
  onClose: () => void;
  refresh: () => void;
  onCut: () => void;
  onCopy: () => void;
  onPaste: () => void;
  onDialog: (dialog: TableDialogState) => void;
  onShowDesign: () => void;
}

export const TableMenus = ({
  model,
  state,
  onClose,
  refresh,
  onCut,
  onCopy,
  onPaste,
  onDialog,
  onShowDesign,
}: TableMenusProps) => {
  const { t } = useTranslation();
  const [error, setError] = useState("");
  const run = (action: () => void) => {
    try {
      action();
      refresh();
    } catch (e) {
      setError(messageOf(e));
    }
  };
  const alert = (
    <Alert
      open={error !== ""}
      title={t("table.error_title")}
      message={error}
      onClose={() => setError("")}
    />
  );
  if (!state) {
    return alert;
  }
  const anchor = { x: state.x, y: state.y };

  const sortItems = (table: string, column: string, numeric: boolean) => (
    <>
      <MenuItem
        icon={<ArrowDownAZ />}
        onClick={() => run(() => model.sortTable(table, column, false))}
      >
        {t(numeric ? "table.sort_smallest" : "table.sort_a_z")}
      </MenuItem>
      <MenuItem
        icon={<ArrowDownZA />}
        onClick={() => run(() => model.sortTable(table, column, true))}
      >
        {t(numeric ? "table.sort_largest" : "table.sort_z_a")}
      </MenuItem>
    </>
  );

  if (state.type === "dropdown") {
    const { table, column, filtered } = state.marker;
    const items = model.getTableFilterItems(table, column);
    const labels = items.map((i) => i.label);
    const numeric = labels
      .filter((l) => l !== BLANK_LABEL)
      .every((l) => !Number.isNaN(Number(l.replace(/,/g, ""))));
    const operators = numeric ? NUMBER_OPERATORS : TEXT_OPERATORS;
    const conditions = (
      <>
        {operators.map((operator) => (
          <MenuItem
            key={operator}
            onClick={() =>
              onDialog({ kind: "filter", table, column, operator })
            }
          >
            {t(`table.operators.${operator}`)}
          </MenuItem>
        ))}
        {numeric ? (
          <>
            <MenuDivider />
            <MenuItem onClick={() => onDialog({ kind: "top", table, column })}>
              {t("table.top_10")}
            </MenuItem>
            {(["above_average", "below_average"] as const).map((operator) => (
              <MenuItem
                key={operator}
                onClick={() =>
                  run(() =>
                    model.setTableFilter(table, column, {
                      type: "condition",
                      operator,
                      value: "",
                      value2: "",
                    }),
                  )
                }
              >
                {t(`table.operators.${operator}`)}
              </MenuItem>
            ))}
          </>
        ) : null}
      </>
    );
    return (
      <>
        {alert}
        <Menu open onClose={onClose} anchorPosition={anchor}>
          {sortItems(table, column, numeric)}
          <MenuDivider />
          <MenuItem
            icon={<FilterX />}
            disabled={!filtered}
            onClick={() => run(() => model.setTableFilter(table, column, null))}
          >
            {t("table.clear_filter_from", { column })}
          </MenuItem>
          <MenuItemWithSubmenu submenu={conditions}>
            {t(numeric ? "table.number_filters" : "table.text_filters")}
          </MenuItemWithSubmenu>
          <MenuDivider />
          <Checklist
            key={`${table}:${column}`}
            items={labels}
            hidden={items.filter((i) => !i.visible).map((i) => i.label)}
            applyLabel={t("table.apply")}
            searchLabel={t("table.search")}
            selectAllLabel={t("table.select_all")}
            onApply={(hidden) => {
              onClose();
              run(() =>
                model.setTableFilter(
                  table,
                  column,
                  hidden.length === 0
                    ? null
                    : {
                        type: "values",
                        visible: labels.filter((l) => !hidden.includes(l)),
                      },
                ),
              );
            }}
          />
        </Menu>
      </>
    );
  }

  const view = model.getSelectedView();
  const entry = model
    .getTables(view.sheet)
    .find((e: TableEntry) => e.name === state.table);
  if (!entry) {
    return alert;
  }
  const { area } = entry;
  const offset = view.column - area.first_column;
  const columnName = entry.columns[offset]?.name ?? "";
  const firstData = area.first_row + (entry.options.header_row ? 1 : 0);
  const lastData = area.last_row - (entry.options.total_row ? 1 : 0);
  const inData = view.row >= firstData && view.row <= lastData;
  const cellText = model.getFormattedCellValue(
    view.sheet,
    view.row,
    view.column,
  );
  const letters = columnLetters(view.column);
  const numericColumn = !Number.isNaN(Number(cellText)) && cellText !== "";
  const name = entry.name;

  return (
    <>
      {alert}
      <Menu open onClose={onClose} anchorPosition={anchor}>
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
              <MenuItem
                disabled={!inData}
                onClick={() => run(() => model.insertTableRow(name, view.row))}
              >
                {t("table.row_above")}
              </MenuItem>
              <MenuItem
                disabled={!inData}
                onClick={() =>
                  run(() => model.insertTableRow(name, view.row + 1))
                }
              >
                {t("table.row_below")}
              </MenuItem>
              <MenuDivider />
              <MenuItem
                onClick={() =>
                  run(() => model.insertTableColumn(name, view.column))
                }
              >
                {t("table.column_left")}
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
              <MenuItem
                disabled={!inData}
                onClick={() => run(() => model.deleteTableRow(name, view.row))}
              >
                {t("table.delete_row", { row: view.row })}
              </MenuItem>
              <MenuItem
                disabled={area.first_column === area.last_column}
                onClick={() =>
                  run(() => model.deleteTableColumn(name, view.column))
                }
              >
                {t("table.delete_column", { column: letters })}
              </MenuItem>
            </>
          }
        >
          {t("context_menu.cell.delete")}
        </MenuItemWithSubmenu>
        <MenuDivider />
        <MenuItemWithSubmenu
          submenu={
            <>
              <MenuItem
                disabled={!inData}
                onClick={() =>
                  run(() =>
                    model.setTableFilter(name, columnName, {
                      type: "values",
                      visible: [cellText === "" ? BLANK_LABEL : cellText],
                    }),
                  )
                }
              >
                {t("table.filter_cell_value", { value: cellText })}
              </MenuItem>
              <MenuItem
                disabled={!entry.filtered}
                onClick={() => run(() => model.clearTableFilters(name))}
              >
                {t("table.clear_all_filters")}
              </MenuItem>
            </>
          }
        >
          {t("table.filter")}
        </MenuItemWithSubmenu>
        <MenuItemWithSubmenu
          submenu={sortItems(name, columnName, numericColumn)}
        >
          {t("table.sort")}
        </MenuItemWithSubmenu>
        <MenuItemWithSubmenu
          icon={<Table2 />}
          submenu={
            <>
              <MenuItem
                onClick={() => run(() => model.convertTableToRange(name))}
              >
                {t("table.convert_to_range")}
              </MenuItem>
              <MenuItem
                checked={entry.options.total_row}
                onClick={() =>
                  run(() =>
                    model.setTableOptions(name, {
                      ...entry.options,
                      total_row: !entry.options.total_row,
                    }),
                  )
                }
              >
                {t("table.total_row")}
              </MenuItem>
              <MenuDivider />
              <MenuItem onClick={onShowDesign}>{t("table.design")}</MenuItem>
            </>
          }
        >
          {t("table.table")}
        </MenuItemWithSubmenu>
      </Menu>
    </>
  );
};

function columnLetters(column: number): string {
  let name = "";
  let rest = column;
  while (rest > 0) {
    const digit = (rest - 1) % 26;
    name = String.fromCharCode(65 + digit) + name;
    rest = Math.floor((rest - 1) / 26);
  }
  return name;
}

interface TableDialogsProps {
  model: Model;
  state: TableDialogState | null;
  onClose: () => void;
  refresh: () => void;
  getSelectedArea: () => string;
}

// Remounted for each opened dialog, so every field starts afresh
export const TableDialogs = (props: TableDialogsProps) => {
  if (!props.state) {
    return null;
  }
  return <TableDialog key={JSON.stringify(props.state)} {...props} />;
};

const TableDialog = ({
  model,
  state,
  onClose,
  refresh,
  getSelectedArea,
}: TableDialogsProps) => {
  const { t } = useTranslation();
  const initialRange = (): string => {
    if (state?.kind === "create") {
      return suggestSource(model, getSelectedArea);
    }
    if (state?.kind === "resize") {
      return (
        model
          .getTables(model.getSelectedSheet())
          .find((e) => e.name === state.table)?.reference ?? ""
      );
    }
    return "";
  };
  const [range, setRange] = useState(initialRange);
  const [hasHeaders, setHasHeaders] = useState(true);
  const fields = (): string[] => {
    if (state?.kind === "duplicates") {
      return (
        model
          .getTables(model.getSelectedSheet())
          .find((e) => e.name === state.table)
          ?.columns.map((c) => c.name) ?? []
      );
    }
    if (state?.kind !== "slicers") {
      return [];
    }
    if (state.table !== null) {
      return (
        model
          .getTables(model.getSelectedSheet())
          .find((e) => e.name === state.table)
          ?.columns.map((c) => c.name) ?? []
      );
    }
    const entry = model.getPivotTables().find((e) => e.index === state.pivot);
    if (!entry || entry.source_sheet === null) {
      return [];
    }
    return model
      .getPivotFields(entry.source_sheet, entry.pivot.source)
      .map((f) => f.name);
  };
  const available = fields();
  const [chosen, setChosen] = useState<string[]>(() =>
    state?.kind === "duplicates" ? available : [],
  );
  const filterRule =
    state?.kind === "filter" || state?.kind === "top"
      ? model.getTableFilter(state.table, state.column)
      : null;
  const existing = filterRule?.type === "condition" ? filterRule : null;
  const [operator, setOperator] = useState<TableFilterOperator>(
    state?.kind === "filter" ? state.operator : "equals",
  );
  const [value, setValue] = useState(existing?.value ?? "");
  const [value2, setValue2] = useState(existing?.value2 ?? "");
  const [count, setCount] = useState("10");
  const [bottom, setBottom] = useState("top");
  const [unit, setUnit] = useState("items");
  const [error, setError] = useState("");
  const [result, setResult] = useState("");

  if (!state) {
    return null;
  }

  const attempt = (action: () => void) => {
    try {
      action();
      refresh();
      onClose();
    } catch (e) {
      setError(messageOf(e));
    }
  };

  const areaOfRange = () => {
    const parsed = parseRangeInSheet(model, range.trim());
    if (!parsed) {
      throw new Error(t("table.invalid_range"));
    }
    const [sheet, first_row, first_column, last_row, last_column] = parsed;
    return { sheet, area: { first_row, first_column, last_row, last_column } };
  };

  const fieldList = (
    <div className="ic-table-field-list">
      {available.map((field) => (
        <Checkbox
          key={field}
          label={field}
          checked={chosen.includes(field)}
          onChange={(on) =>
            setChosen(
              on ? [...chosen, field] : chosen.filter((f) => f !== field),
            )
          }
        />
      ))}
    </div>
  );
  const errorLine = error ? (
    <div className="ic-pivot-error">{error}</div>
  ) : null;

  if (result !== "") {
    return (
      <Alert
        open
        title={t("table.remove_duplicates")}
        message={result}
        onClose={onClose}
      />
    );
  }

  switch (state.kind) {
    case "create":
      return (
        <DialogShell
          title={t("table.create_title")}
          onClose={onClose}
          onConfirm={() =>
            attempt(() => {
              const { sheet, area } = areaOfRange();
              model.createTable(sheet, area, hasHeaders, null);
              model.setSelectedSheet(sheet);
            })
          }
        >
          <div className="ic-pivot-label">{t("table.where_is_the_data")}</div>
          <Input
            type="text"
            size="sm"
            value={range}
            aria-label={t("table.where_is_the_data")}
            onChange={(e) => setRange(e.target.value)}
          />
          <Checkbox
            label={t("table.has_headers")}
            checked={hasHeaders}
            onChange={setHasHeaders}
          />
          {errorLine}
        </DialogShell>
      );
    case "resize":
      return (
        <DialogShell
          title={t("table.resize")}
          onClose={onClose}
          onConfirm={() =>
            attempt(() => model.resizeTable(state.table, areaOfRange().area))
          }
        >
          <div className="ic-pivot-label">{t("table.new_range")}</div>
          <Input
            type="text"
            size="sm"
            value={range}
            aria-label={t("table.new_range")}
            onChange={(e) => setRange(e.target.value)}
          />
          {errorLine}
        </DialogShell>
      );
    case "slicers":
      return (
        <DialogShell
          title={t("table.insert_slicers")}
          onClose={onClose}
          confirmDisabled={chosen.length === 0}
          onConfirm={() =>
            attempt(() => {
              if (state.table !== null) {
                model.createTableSlicers(state.table, chosen);
              } else if (state.pivot !== null) {
                model.createPivotSlicers(state.pivot, chosen);
              }
            })
          }
        >
          {fieldList}
          {errorLine}
        </DialogShell>
      );
    case "duplicates":
      return (
        <DialogShell
          title={t("table.remove_duplicates")}
          onClose={onClose}
          confirmDisabled={chosen.length === 0}
          onConfirm={() => {
            try {
              const [removed, remaining] = model.removeTableDuplicates(
                state.table,
                chosen,
              );
              refresh();
              setResult(t("table.duplicates_result", { removed, remaining }));
            } catch (e) {
              setError(messageOf(e));
            }
          }}
        >
          <div className="ic-pivot-hint">{t("table.duplicates_hint")}</div>
          {fieldList}
          {errorLine}
        </DialogShell>
      );
    case "top": {
      const countValue = Number(count);
      return (
        <DialogShell
          title={t("table.top_10_title", { column: state.column })}
          onClose={onClose}
          confirmDisabled={!(countValue > 0)}
          onConfirm={() =>
            attempt(() =>
              model.setTableFilter(state.table, state.column, {
                type: "top",
                count: countValue,
                bottom: bottom === "bottom",
                percent: unit === "percent",
              }),
            )
          }
        >
          <div className="ic-table-dialog-row">
            <Select
              size="sm"
              value={bottom}
              options={[
                { value: "top", label: t("table.top") },
                { value: "bottom", label: t("table.bottom") },
              ]}
              onChange={setBottom}
            />
            <Input
              type="text"
              inputMode="decimal"
              size="sm"
              value={count}
              aria-label={t("table.top_10")}
              onChange={(e) => setCount(e.target.value)}
            />
            <Select
              size="sm"
              value={unit}
              options={[
                { value: "items", label: t("table.items") },
                { value: "percent", label: t("table.percent") },
              ]}
              onChange={setUnit}
            />
          </div>
          {errorLine}
        </DialogShell>
      );
    }
    case "filter": {
      const choices = [...TEXT_OPERATORS, ...NUMBER_OPERATORS].filter(
        (o, i, all) => all.indexOf(o) === i,
      );
      const twoValues = operator === "between" || operator === "not_between";
      return (
        <DialogShell
          title={t("table.custom_filter_title", { column: state.column })}
          onClose={onClose}
          confirmDisabled={value === "" || (twoValues && value2 === "")}
          onConfirm={() =>
            attempt(() =>
              model.setTableFilter(state.table, state.column, {
                type: "condition",
                operator,
                value,
                value2: twoValues ? value2 : "",
              }),
            )
          }
        >
          <Select
            size="sm"
            value={operator}
            options={choices.map((o) => ({
              value: o,
              label: t(`table.operators.${o}`),
            }))}
            onChange={(next) => setOperator(next as TableFilterOperator)}
          />
          <Input
            type="text"
            size="sm"
            value={value}
            aria-label={t("table.value")}
            onChange={(e) => setValue(e.target.value)}
          />
          {twoValues ? (
            <Input
              type="text"
              size="sm"
              value={value2}
              aria-label={t("table.value")}
              onChange={(e) => setValue2(e.target.value)}
            />
          ) : null}
          {errorLine}
        </DialogShell>
      );
    }
  }
};
