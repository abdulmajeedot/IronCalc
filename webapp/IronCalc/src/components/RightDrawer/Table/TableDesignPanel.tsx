import type { Model, TableEntry, TableOptions } from "@ironcalc/wasm";
import { X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../Button/Button";
import { IconButton } from "../../Button/IconButton";
import { Checkbox } from "../../Checkbox/Checkbox";
import { Input } from "../../Input/Input";
import { Select } from "../../Select/Select";
import { Tooltip } from "../../Tooltip/Tooltip";
import { messageOf, type TableDialogState } from "./TableMenus";
import "./table.css";

const STYLE_GROUPS: [string, number][] = [
  ["Light", 21],
  ["Medium", 28],
  ["Dark", 11],
];

const TOTAL_FUNCTIONS = [
  "none",
  "average",
  "count",
  "countNums",
  "max",
  "min",
  "sum",
  "stdDev",
  "var",
] as const;

interface SwatchProps {
  model: Model;
  name: string | null;
  selected: boolean;
  label: string;
  onPick: () => void;
}

// A small picture of a table in one of the styles.
const StyleSwatch = ({ model, name, selected, label, onPick }: SwatchProps) => {
  const swatch = name ? model.getTableStyleSwatch(name) : null;
  const line = swatch?.line ?? "var(--palette-grey-300)";
  const rows = [0, 1, 2, 3];
  return (
    <button
      type="button"
      className={`ic-table-swatch${selected ? " ic-table-swatch--selected" : ""}`}
      aria-label={label}
      aria-pressed={selected}
      title={label}
      onClick={onPick}
    >
      <span
        className="ic-table-swatch-row"
        style={{
          background: swatch?.header_fill ?? "transparent",
          borderBottom: `1px solid ${line}`,
        }}
      />
      {rows.map((row) => (
        <span
          key={row}
          className="ic-table-swatch-row"
          style={{
            background:
              (row % 2 === 0 ? swatch?.stripe_fill : swatch?.body_fill) ??
              swatch?.body_fill ??
              "transparent",
            borderBottom: `1px solid ${line}`,
          }}
        />
      ))}
    </button>
  );
};

interface TableDesignPanelProps {
  model: Model;
  onClose: () => void;
  onUpdate: () => void;
  onDialog: (dialog: TableDialogState) => void;
}

const TableDesignPanel = ({
  model,
  onClose,
  onUpdate,
  onDialog,
}: TableDesignPanelProps) => {
  const { t } = useTranslation();
  const [error, setError] = useState("");
  const view = model.getSelectedView();
  const entry: TableEntry | null = model.getTableAt(
    view.sheet,
    view.row,
    view.column,
  );

  const header = (
    <div className="ic-pivot-header">
      <div className="ic-pivot-title">{t("table.design")}</div>
      <Tooltip title={t("right_drawer.close")}>
        <IconButton
          icon={<X />}
          onClick={onClose}
          aria-label={t("right_drawer.close")}
        />
      </Tooltip>
    </div>
  );
  if (!entry) {
    return (
      <div className="ic-pivot-container">
        {header}
        <div className="ic-pivot-body">
          <div className="ic-pivot-hint">{t("table.select_a_table")}</div>
          <Button size="sm" onClick={() => onDialog({ kind: "create" })}>
            {t("table.create_title")}
          </Button>
        </div>
      </div>
    );
  }

  const run = (action: () => void) => {
    try {
      action();
      setError("");
      onUpdate();
    } catch (e) {
      setError(messageOf(e));
    }
  };
  const apply = (patch: Partial<TableOptions>) =>
    run(() =>
      model.setTableOptions(entry.name, { ...entry.options, ...patch }),
    );
  const toggle = (key: keyof TableOptions, label: string) => (
    <Checkbox
      label={label}
      checked={entry.options[key] === true}
      onChange={(on) => apply({ [key]: on })}
    />
  );

  return (
    <div className="ic-pivot-container">
      {header}
      <div className="ic-pivot-body">
        {error ? <div className="ic-pivot-error">{error}</div> : null}
        <div className="ic-pivot-label">{t("table.table_name")}</div>
        <Input
          key={entry.name}
          type="text"
          size="sm"
          defaultValue={entry.name}
          aria-label={t("table.table_name")}
          onKeyDown={(event) => {
            event.stopPropagation();
            if (event.key === "Enter") {
              event.currentTarget.blur();
            }
          }}
          onBlur={(event) => {
            const name = event.target.value.trim();
            if (name !== entry.name) {
              apply({ name });
            }
          }}
        />
        <div className="ic-table-design-buttons">
          <Button
            size="sm"
            variant="secondary"
            onClick={() =>
              onDialog({ kind: "slicers", table: entry.name, pivot: null })
            }
          >
            {t("table.insert_slicer")}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => onDialog({ kind: "resize", table: entry.name })}
          >
            {t("table.resize")}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => onDialog({ kind: "duplicates", table: entry.name })}
          >
            {t("table.remove_duplicates")}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => run(() => model.convertTableToRange(entry.name))}
          >
            {t("table.convert_to_range")}
          </Button>
        </div>
        <div className="ic-pivot-section-title">{t("table.style_options")}</div>
        {toggle("header_row", t("table.header_row"))}
        {toggle("total_row", t("table.total_row"))}
        {toggle("banded_rows", t("table.banded_rows"))}
        {toggle("first_column", t("table.first_column"))}
        {toggle("last_column", t("table.last_column"))}
        {toggle("banded_columns", t("table.banded_columns"))}
        {toggle("filter_button", t("table.filter_button"))}
        {entry.options.total_row ? (
          <>
            <div className="ic-pivot-section-title">{t("table.total_row")}</div>
            {entry.columns.map((column) => (
              <div className="ic-table-total-row" key={column.name}>
                <span className="ic-table-total-name">{column.name}</span>
                <Select
                  size="sm"
                  value={column.totals_function ?? "none"}
                  options={TOTAL_FUNCTIONS.map((f) => ({
                    value: f,
                    label: t(`table.totals.${f}`),
                  }))}
                  onChange={(next) =>
                    run(() =>
                      model.setTableTotalFunction(
                        entry.name,
                        column.name,
                        next === "none" ? null : next,
                      ),
                    )
                  }
                />
              </div>
            ))}
          </>
        ) : null}
        <div className="ic-pivot-section-title">{t("table.table_styles")}</div>
        <div className="ic-table-swatches">
          <StyleSwatch
            model={model}
            name={null}
            selected={entry.options.style_name === null}
            label={t("table.no_style")}
            onPick={() => apply({ style_name: null })}
          />
        </div>
        {STYLE_GROUPS.map(([group, count]) => (
          <div key={group}>
            <div className="ic-pivot-label">
              {t(`table.style_groups.${group.toLowerCase()}`)}
            </div>
            <div className="ic-table-swatches">
              {Array.from({ length: count }, (_, i) => {
                const name = `TableStyle${group}${i + 1}`;
                return (
                  <StyleSwatch
                    key={name}
                    model={model}
                    name={name}
                    selected={entry.options.style_name === name}
                    label={`${group} ${i + 1}`}
                    onPick={() => apply({ style_name: name })}
                  />
                );
              })}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
};

export default TableDesignPanel;
