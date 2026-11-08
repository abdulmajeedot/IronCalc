import type {
  Model,
  PivotAggregation,
  PivotField,
  PivotFieldInfo,
  PivotTable,
  PivotTableEntry,
  PivotValue,
} from "@ironcalc/wasm";
import {
  ArrowDown,
  ArrowUp,
  ChevronDown,
  ChevronUp,
  Columns3,
  Filter,
  RefreshCw,
  Rows3,
  Search,
  Sigma,
  Trash2,
  X,
} from "lucide-react";
import type { DragEvent, ReactNode } from "react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../Button/Button";
import { IconButton } from "../../Button/IconButton";
import { Checkbox } from "../../Checkbox/Checkbox";
import { parseRangeInSheet } from "../../Editor/util";
import { Input } from "../../Input/Input";
import { Select } from "../../Select/Select";
import { Tooltip } from "../../Tooltip/Tooltip";
import {
  AGGREGATIONS,
  ALL_ZONES,
  addField,
  currentRegion,
  isFieldUsed,
  moveWithin,
  nextSheetName,
  type PivotZone,
  placeField,
  removeAt,
  removeField,
} from "./pivotLogic";
import "./pivot-table.css";

const OVERWRITE_ERROR = "PIVOT_OVERWRITE";
const FIRST_OUTPUT_ROW = 3;
const FIRST_OUTPUT_COLUMN = 1;
const LAST_ROW = 1048576;
const LAST_COLUMN = 16384;

const messageOf = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

const ZONE_ICONS: Record<PivotZone, ReactNode> = {
  filters: <Filter />,
  rows: <Rows3 />,
  columns: <Columns3 />,
  values: <Sigma />,
};

interface PivotTablePanelProps {
  onClose: () => void;
  model: Model;
  onUpdate: () => void;
  getSelectedArea: () => string;
}

interface CreateFormProps extends PivotTablePanelProps {
  defaultSource: string;
}

// The block of data a pivot should start from, as a sheet-qualified range.
function suggestSource(model: Model, getSelectedArea: () => string): string {
  const view = model.getSelectedView();
  const [rowStart, columnStart, rowEnd, columnEnd] = view.range;
  const isWholeSheetAxis = rowEnd === LAST_ROW || columnEnd === LAST_COLUMN;
  const isSingleCell = rowStart === rowEnd && columnStart === columnEnd;
  if (!isSingleCell && !isWholeSheetAxis) {
    return getSelectedArea();
  }
  const region = currentRegion(
    (row, column) =>
      model.getFormattedCellValue(view.sheet, row, column) !== "",
    rowStart,
    columnStart,
  );
  const name = model.getWorksheetsProperties()[view.sheet].name;
  const quoted = /^[A-Za-z_][A-Za-z0-9_]*$/.test(name)
    ? name
    : `'${name.replace(/'/g, "''")}'`;
  return `${quoted}!${columnName(region.first_column)}${region.first_row}:${columnName(region.last_column)}${region.last_row}`;
}

function columnName(column: number): string {
  let name = "";
  let rest = column;
  while (rest > 0) {
    const digit = (rest - 1) % 26;
    name = String.fromCharCode(65 + digit) + name;
    rest = Math.floor((rest - 1) / 26);
  }
  return name;
}

const CreateForm = ({
  model,
  onClose,
  onUpdate,
  defaultSource,
}: CreateFormProps) => {
  const { t } = useTranslation();
  const [source, setSource] = useState(defaultSource);
  const [placement, setPlacement] = useState("new");
  const [destination, setDestination] = useState("");
  const [error, setError] = useState("");
  const [confirmReplace, setConfirmReplace] = useState(false);

  const create = (overwrite: boolean) => {
    setError("");
    setConfirmReplace(false);
    const worksheets = model.getWorksheetsProperties();
    const range = parseRangeInSheet(model, source.trim());
    if (!range) {
      setError(t("pivot_table.invalid_source"));
      return;
    }
    const [sheet, first_row, first_column, last_row, last_column] = range;
    let target = {
      sheet_id: worksheets[sheet].sheet_id,
      row: FIRST_OUTPUT_ROW,
      column: FIRST_OUTPUT_COLUMN,
    };
    let newSheet: string | null = nextSheetName(
      worksheets.map((w) => w.name),
      t("pivot_table.new_sheet_name"),
    );
    if (placement === "existing") {
      const cell = parseRangeInSheet(model, destination.trim());
      if (!cell) {
        setError(t("pivot_table.invalid_destination"));
        return;
      }
      target = {
        sheet_id: worksheets[cell[0]].sheet_id,
        row: cell[1],
        column: cell[2],
      };
      newSheet = null;
    }
    const pivot: PivotTable = {
      name: "",
      ...target,
      source_sheet_id: worksheets[sheet].sheet_id,
      source: { first_row, first_column, last_row, last_column },
      rows: [],
      columns: [],
      values: [],
      filters: [],
      options: {
        grand_totals_rows: true,
        grand_totals_columns: true,
        subtotals: "top",
        layout: "compact",
        repeat_labels: false,
      },
      output: null,
      output_map: null,
      style_name: "",
      read_only: false,
    };
    try {
      model.getPivotFields(sheet, pivot.source);
      model.createPivotTable(pivot, newSheet, overwrite);
      const after = model.getWorksheetsProperties();
      const outputSheet = after.findIndex(
        (w) => w.sheet_id === target.sheet_id,
      );
      model.setSelectedSheet(
        newSheet === null ? outputSheet : after.length - 1,
      );
      model.setSelectedCell(target.row, target.column);
      onUpdate();
    } catch (e) {
      const message = messageOf(e);
      if (message === OVERWRITE_ERROR) {
        setConfirmReplace(true);
      } else {
        setError(message);
      }
    }
  };

  return (
    <div className="ic-pivot-container">
      <div className="ic-pivot-header">
        <div className="ic-pivot-title">{t("pivot_table.create_title")}</div>
        <Tooltip title={t("right_drawer.close")}>
          <IconButton
            icon={<X />}
            onClick={onClose}
            aria-label={t("right_drawer.close")}
          />
        </Tooltip>
      </div>
      <div className="ic-pivot-body">
        <div className="ic-pivot-hint">{t("pivot_table.create_hint")}</div>
        <div className="ic-pivot-label">{t("pivot_table.source")}</div>
        <Input
          type="text"
          size="sm"
          value={source}
          onChange={(e) => setSource(e.target.value)}
          aria-label={t("pivot_table.source")}
        />
        <div className="ic-pivot-label">{t("pivot_table.destination")}</div>
        <Select
          size="sm"
          value={placement}
          options={[
            { value: "new", label: t("pivot_table.new_sheet") },
            { value: "existing", label: t("pivot_table.existing_sheet") },
          ]}
          onChange={setPlacement}
        />
        {placement === "existing" ? (
          <Input
            type="text"
            size="sm"
            value={destination}
            placeholder={t("pivot_table.destination_placeholder")}
            onChange={(e) => setDestination(e.target.value)}
            aria-label={t("pivot_table.destination")}
          />
        ) : null}
        {error ? <div className="ic-pivot-error">{error}</div> : null}
        {confirmReplace ? (
          <div className="ic-pivot-error">
            {t("pivot_table.replace_warning")}
            <div className="ic-pivot-error-actions">
              <Button size="sm" onClick={() => create(true)}>
                {t("pivot_table.replace")}
              </Button>
            </div>
          </div>
        ) : null}
      </div>
      <div className="ic-pivot-footer">
        <Button onClick={() => create(false)}>{t("pivot_table.create")}</Button>
      </div>
    </div>
  );
};

interface ChipProps {
  label: string;
  expanded: boolean;
  onToggle: () => void;
  onDragStart: () => void;
  onDropBefore: () => void;
  children: ReactNode;
}

const Chip = ({
  label,
  expanded,
  onToggle,
  onDragStart,
  onDropBefore,
  children,
}: ChipProps) => {
  const { t } = useTranslation();
  return (
    <div className="ic-pivot-chip-wrapper">
      {/* biome-ignore lint/a11y/noStaticElementInteractions: a drag handle; the menu button is the keyboard route */}
      <div
        className="ic-pivot-chip"
        draggable
        onDragStart={(e: DragEvent) => {
          e.dataTransfer.effectAllowed = "move";
          e.dataTransfer.setData("text/plain", label);
          onDragStart();
        }}
        onDragOver={(e: DragEvent) => e.preventDefault()}
        onDrop={(e: DragEvent) => {
          e.preventDefault();
          e.stopPropagation();
          onDropBefore();
        }}
      >
        <span className="ic-pivot-chip-label" title={label}>
          {label}
        </span>
        <IconButton
          icon={expanded ? <ChevronUp /> : <ChevronDown />}
          aria-label={t("pivot_table.field_options", { field: label })}
          aria-expanded={expanded}
          onClick={onToggle}
        />
      </div>
      {expanded ? <div className="ic-pivot-chip-menu">{children}</div> : null}
    </div>
  );
};

interface EditorProps extends PivotTablePanelProps {
  entry: PivotTableEntry;
}

const PivotEditor = ({ entry, model, onClose, onUpdate }: EditorProps) => {
  const { t } = useTranslation();
  const { pivot, index, source_sheet: sourceSheet } = entry;
  const readOnly = pivot.read_only;
  const [search, setSearch] = useState("");
  const [expanded, setExpanded] = useState<string | null>(null);
  const [dragging, setDragging] = useState<{
    field: string;
    zone?: PivotZone;
    index?: number;
  } | null>(null);
  const [error, setError] = useState("");
  const [pending, setPending] = useState<PivotTable | null>(null);
  const [version, setVersion] = useState(0);
  const sourceKey = JSON.stringify(pivot.source);

  // biome-ignore lint/correctness/useExhaustiveDependencies: the fields only change with the source or a refresh
  const fieldsResult = useMemo((): {
    fields: PivotFieldInfo[];
    error: string;
  } => {
    if (sourceSheet === null) {
      return { fields: [], error: t("pivot_table.source_missing") };
    }
    try {
      return {
        fields: model.getPivotFields(sourceSheet, pivot.source),
        error: "",
      };
    } catch (e) {
      return { fields: [], error: messageOf(e) };
    }
  }, [model, sourceSheet, sourceKey, version]);
  const { fields } = fieldsResult;
  const infoOf = (name: string) => fields.find((f) => f.name === name);

  const apply = (next: PivotTable, overwrite = false) => {
    setError("");
    setPending(null);
    try {
      model.updatePivotTable(index, next, overwrite);
      onUpdate();
    } catch (e) {
      const message = messageOf(e);
      if (message === OVERWRITE_ERROR) {
        setPending(next);
      } else {
        setError(message);
      }
    }
  };

  const refresh = () => {
    setError("");
    try {
      model.refreshPivotTable(index);
      setVersion((v) => v + 1);
      onUpdate();
    } catch (e) {
      setError(messageOf(e));
    }
  };

  const remove = () => {
    try {
      model.deletePivotTable(index);
      onUpdate();
      onClose();
    } catch (e) {
      setError(messageOf(e));
    }
  };

  const dropOn = (zone: PivotZone, at: number) => {
    if (!dragging || readOnly) {
      setDragging(null);
      return;
    }
    const numeric = infoOf(dragging.field)?.numeric ?? false;
    let position = at;
    if (
      dragging.zone === zone &&
      dragging.index !== undefined &&
      dragging.index < at
    ) {
      position -= 1;
    }
    setDragging(null);
    apply(placeField(pivot, dragging.field, numeric, zone, position));
  };

  const zoneTitle: Record<PivotZone, string> = {
    filters: t("pivot_table.filters"),
    rows: t("pivot_table.rows"),
    columns: t("pivot_table.columns"),
    values: t("pivot_table.values"),
  };
  const aggregationOptions = AGGREGATIONS.map((value) => ({
    value,
    label: t(`pivot_table.aggregations.${value}`),
  }));

  const moveMenu = (zone: PivotZone, position: number, field: string) => (
    <>
      <div className="ic-pivot-menu-row">
        <Tooltip title={t("pivot_table.move_up")}>
          <IconButton
            icon={<ArrowUp />}
            aria-label={t("pivot_table.move_up")}
            onClick={() => apply(moveWithin(pivot, zone, position, -1))}
            disabled={position === 0}
          />
        </Tooltip>
        <Tooltip title={t("pivot_table.move_down")}>
          <IconButton
            icon={<ArrowDown />}
            aria-label={t("pivot_table.move_down")}
            onClick={() => apply(moveWithin(pivot, zone, position, 1))}
            disabled={position === pivot[zone].length - 1}
          />
        </Tooltip>
        <Tooltip title={t("pivot_table.remove_field")}>
          <IconButton
            icon={<Trash2 />}
            aria-label={t("pivot_table.remove_field")}
            onClick={() => {
              setExpanded(null);
              apply(removeAt(pivot, zone, position));
            }}
          />
        </Tooltip>
      </div>
      <Select
        size="sm"
        aria-label={t("pivot_table.move_to")}
        value={zone}
        options={ALL_ZONES.map((z) => ({ value: z, label: zoneTitle[z] }))}
        onChange={(target) => {
          if (target === zone) {
            return;
          }
          setExpanded(null);
          apply(
            placeField(
              pivot,
              field,
              infoOf(field)?.numeric ?? false,
              target as PivotZone,
              pivot[target as PivotZone].length,
            ),
          );
        }}
      />
    </>
  );

  const updateAxis = (
    zone: "filters" | "rows" | "columns",
    position: number,
    next: PivotField,
  ) => {
    const list = [...pivot[zone]];
    list[position] = next;
    apply({ ...pivot, [zone]: list });
  };

  const itemFilter = (
    zone: "filters" | "rows" | "columns",
    position: number,
    field: PivotField,
  ) => {
    const items = infoOf(field.field)?.items ?? [];
    return (
      <div className="ic-pivot-item-list">
        {items.map((item) => (
          <Checkbox
            key={item}
            label={item}
            checked={!field.hidden_items.includes(item)}
            onChange={(visible) => {
              const hidden = visible
                ? field.hidden_items.filter((i) => i !== item)
                : [...field.hidden_items, item];
              if (hidden.length >= items.length) {
                setError(t("pivot_table.keep_one_item"));
                return;
              }
              updateAxis(zone, position, { ...field, hidden_items: hidden });
            }}
          />
        ))}
      </div>
    );
  };

  const updateValue = (position: number, next: PivotValue) => {
    const values = [...pivot.values];
    values[position] = next;
    apply({ ...pivot, values });
  };

  const renderChips = (zone: PivotZone) => {
    if (zone === "values") {
      return pivot.values.map((value, position) => {
        const key = `values:${value.field}:${position}`;
        const label =
          value.name ||
          t("pivot_table.value_caption", {
            aggregation: t(`pivot_table.aggregations.${value.aggregation}`),
            field: value.field,
          });
        return (
          <Chip
            key={key}
            label={label}
            expanded={expanded === key}
            onToggle={() => setExpanded(expanded === key ? null : key)}
            onDragStart={() =>
              setDragging({ field: value.field, zone, index: position })
            }
            onDropBefore={() => dropOn(zone, position)}
          >
            <div className="ic-pivot-label">{t("pivot_table.custom_name")}</div>
            <Input
              key={value.name}
              type="text"
              size="sm"
              defaultValue={value.name}
              placeholder={label}
              aria-label={t("pivot_table.custom_name")}
              onBlur={(e) => {
                const name = e.target.value.trim();
                if (name !== value.name) {
                  if (name.toLowerCase() === value.field.toLowerCase()) {
                    setError(t("pivot_table.name_taken"));
                    return;
                  }
                  updateValue(position, { ...value, name });
                }
              }}
            />
            <div className="ic-pivot-label">
              {t("pivot_table.summarize_by")}
            </div>
            <Select
              size="sm"
              aria-label={t("pivot_table.summarize_by")}
              value={value.aggregation}
              options={aggregationOptions}
              onChange={(aggregation) =>
                updateValue(position, {
                  ...value,
                  aggregation: aggregation as PivotAggregation,
                })
              }
            />
            {moveMenu(zone, position, value.field)}
          </Chip>
        );
      });
    }
    return pivot[zone].map((field, position) => {
      const key = `${zone}:${field.field}`;
      return (
        <Chip
          key={key}
          label={field.field}
          expanded={expanded === key}
          onToggle={() => setExpanded(expanded === key ? null : key)}
          onDragStart={() =>
            setDragging({ field: field.field, zone, index: position })
          }
          onDropBefore={() => dropOn(zone, position)}
        >
          {moveMenu(zone, position, field.field)}
          <div className="ic-pivot-label">{t("pivot_table.show_items")}</div>
          {itemFilter(zone, position, field)}
        </Chip>
      );
    });
  };

  const options = pivot.options;
  // A setting for the whole pivot replaces what single fields had overridden
  const setOptions = (changes: Partial<typeof options>) => {
    const clear = (list: PivotField[]) =>
      list.map((field) => ({
        ...field,
        subtotal: changes.subtotals ? null : field.subtotal,
        repeat_labels:
          changes.repeat_labels === undefined ? field.repeat_labels : null,
      }));
    apply({
      ...pivot,
      rows: clear(pivot.rows),
      columns: clear(pivot.columns),
      options: { ...options, ...changes },
    });
  };
  const query = search.trim().toLowerCase();
  const visibleFields = fields.filter((f) =>
    f.name.toLowerCase().includes(query),
  );

  return (
    <div className="ic-pivot-container">
      <div className="ic-pivot-header">
        <div className="ic-pivot-title">{pivot.name}</div>
        <Tooltip title={t("pivot_table.refresh")}>
          <IconButton
            icon={<RefreshCw />}
            onClick={refresh}
            disabled={readOnly}
            aria-label={t("pivot_table.refresh")}
          />
        </Tooltip>
        <Tooltip title={t("pivot_table.delete")}>
          <IconButton
            icon={<Trash2 />}
            onClick={remove}
            aria-label={t("pivot_table.delete")}
          />
        </Tooltip>
        <Tooltip title={t("right_drawer.close")}>
          <IconButton
            icon={<X />}
            onClick={onClose}
            aria-label={t("right_drawer.close")}
          />
        </Tooltip>
      </div>
      <div className="ic-pivot-body">
        {readOnly ? (
          <div className="ic-pivot-notice">{t("pivot_table.read_only")}</div>
        ) : null}
        {fieldsResult.error ? (
          <div className="ic-pivot-error">{fieldsResult.error}</div>
        ) : null}
        {error ? <div className="ic-pivot-error">{error}</div> : null}
        {pending ? (
          <div className="ic-pivot-error">
            {t("pivot_table.replace_warning")}
            <div className="ic-pivot-error-actions">
              <Button size="sm" onClick={() => apply(pending, true)}>
                {t("pivot_table.replace")}
              </Button>
            </div>
          </div>
        ) : null}

        <Input
          type="text"
          size="sm"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder={t("pivot_table.search_fields")}
          startAdornment={<Search />}
        />
        <div className="ic-pivot-field-list">
          {visibleFields.map((info) => (
            // biome-ignore lint/a11y/noStaticElementInteractions: a drag handle around the checkbox
            <div
              key={info.name}
              className="ic-pivot-field"
              draggable={!readOnly}
              onDragStart={(e: DragEvent) => {
                e.dataTransfer.effectAllowed = "move";
                e.dataTransfer.setData("text/plain", info.name);
                setDragging({ field: info.name });
              }}
            >
              <Checkbox
                label={info.name}
                disabled={readOnly}
                checked={isFieldUsed(pivot, info.name)}
                onChange={(checked) =>
                  apply(
                    checked
                      ? addField(pivot, info)
                      : removeField(pivot, info.name),
                  )
                }
              />
            </div>
          ))}
        </div>

        <div className={readOnly ? "ic-pivot-zones--disabled" : undefined}>
          {ALL_ZONES.map((zone) => (
            // biome-ignore lint/a11y/noStaticElementInteractions: a drop target; each chip has a menu with the same moves
            <div
              key={zone}
              className="ic-pivot-zone"
              onDragOver={(e: DragEvent) => e.preventDefault()}
              onDrop={(e: DragEvent) => {
                e.preventDefault();
                dropOn(zone, pivot[zone].length);
              }}
            >
              <div className="ic-pivot-zone-title">
                {ZONE_ICONS[zone]}
                {zoneTitle[zone]}
              </div>
              <div className="ic-pivot-zone-body">
                {renderChips(zone)}
                {zone === "columns" && pivot.values.length > 1 ? (
                  <div className="ic-pivot-chip ic-pivot-chip--virtual">
                    <span className="ic-pivot-chip-label">
                      {t("pivot_table.values")}
                    </span>
                  </div>
                ) : null}
              </div>
            </div>
          ))}
        </div>

        <div
          className={`ic-pivot-settings${readOnly ? " ic-pivot-zones--disabled" : ""}`}
        >
          <div className="ic-pivot-section-title">
            {t("pivot_table.settings")}
          </div>
          <Checkbox
            label={t("pivot_table.grand_totals_rows")}
            checked={options.grand_totals_rows}
            onChange={(grand_totals_rows) => setOptions({ grand_totals_rows })}
          />
          <Checkbox
            label={t("pivot_table.grand_totals_columns")}
            checked={options.grand_totals_columns}
            onChange={(grand_totals_columns) =>
              setOptions({ grand_totals_columns })
            }
          />
          <div className="ic-pivot-label">{t("pivot_table.subtotals")}</div>
          <Select
            size="sm"
            aria-label={t("pivot_table.subtotals")}
            value={options.subtotals}
            options={[
              { value: "none", label: t("pivot_table.subtotals_none") },
              { value: "top", label: t("pivot_table.subtotals_top") },
              { value: "bottom", label: t("pivot_table.subtotals_bottom") },
            ]}
            onChange={(subtotals) =>
              setOptions({
                subtotals: subtotals as PivotTable["options"]["subtotals"],
              })
            }
          />
          <div className="ic-pivot-label">{t("pivot_table.layout")}</div>
          <Select
            size="sm"
            aria-label={t("pivot_table.layout")}
            value={options.layout}
            options={[
              { value: "compact", label: t("pivot_table.layout_compact") },
              { value: "tabular", label: t("pivot_table.layout_tabular") },
            ]}
            onChange={(layout) =>
              setOptions({ layout: layout as PivotTable["options"]["layout"] })
            }
          />
          {options.layout === "tabular" ? (
            <Checkbox
              label={t("pivot_table.repeat_labels")}
              checked={options.repeat_labels}
              onChange={(repeat_labels) => setOptions({ repeat_labels })}
            />
          ) : null}
        </div>
      </div>
    </div>
  );
};

const PivotTablePanel = (props: PivotTablePanelProps) => {
  const { model, getSelectedArea } = props;
  const view = model.getSelectedView();
  const index = model.getPivotTableAt(view.sheet, view.row, view.column);
  const entry =
    index === undefined
      ? undefined
      : model.getPivotTables().find((e) => e.index === index);
  if (entry) {
    return <PivotEditor {...props} entry={entry} key={entry.index} />;
  }
  return (
    <CreateForm
      {...props}
      defaultSource={suggestSource(model, getSelectedArea)}
      key={`${view.sheet}:${view.range.join(":")}`}
    />
  );
};

export default PivotTablePanel;
