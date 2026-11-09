import type { Model, Slicer, SlicerPlacement } from "@ironcalc/wasm";
import { X } from "lucide-react";
import { type FocusEvent, type KeyboardEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { IconButton } from "../../Button/IconButton";
import { Checkbox } from "../../Checkbox/Checkbox";
import { Input } from "../../Input/Input";
import { Select } from "../../Select/Select";
import { Tooltip } from "../../Tooltip/Tooltip";
import { slicerColors } from "./SlicerLayer";
import { messageOf } from "./TableMenus";
import "./table.css";

const PIXELS_PER_INCH = 96;
const ITEMS_PADDING = 12;
const ITEMS_GAP = 4;

const STYLES = [
  ...[1, 2, 3, 4, 5, 6].map((n) => `SlicerStyleLight${n}`),
  "SlicerStyleOther1",
  "SlicerStyleOther2",
  ...[1, 2, 3, 4, 5, 6].map((n) => `SlicerStyleDark${n}`),
];

interface FieldProps {
  label: string;
  value: number;
  onCommit: (value: number) => void;
  disabled?: boolean;
}

const commitOnEnter = (event: KeyboardEvent<HTMLInputElement>) => {
  event.stopPropagation();
  if (event.key === "Enter") {
    event.currentTarget.blur();
  }
};

// A measure shown in inches, applied when the field is left.
const MeasureField = ({ label, value, onCommit, disabled }: FieldProps) => {
  const shown = (value / PIXELS_PER_INCH).toFixed(2);
  return (
    <div className="ic-table-total-row">
      <span className="ic-table-total-name">{label}</span>
      <Input
        key={shown}
        type="text"
        inputMode="decimal"
        size="sm"
        disabled={disabled}
        defaultValue={`${shown}"`}
        aria-label={label}
        onKeyDown={commitOnEnter}
        onBlur={(event: FocusEvent<HTMLInputElement>) => {
          const parsed = Number.parseFloat(event.target.value);
          if (
            Number.isFinite(parsed) &&
            parsed >= 0 &&
            parsed.toFixed(2) !== shown
          ) {
            onCommit(parsed * PIXELS_PER_INCH);
          }
        }}
      />
    </div>
  );
};

interface SlicerSettingsPanelProps {
  model: Model;
  name: string | null;
  onClose: () => void;
  onUpdate: () => void;
}

const SlicerSettingsPanel = ({
  model,
  name,
  onClose,
  onUpdate,
}: SlicerSettingsPanelProps) => {
  const { t } = useTranslation();
  const [error, setError] = useState("");
  const sheet = model.getSelectedSheet();
  const entry = model.getSlicers(sheet).find((e) => e.slicer.name === name);

  const header = (
    <div className="ic-pivot-header">
      <div className="ic-pivot-title">{t("table.slicer_settings")}</div>
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
          <div className="ic-pivot-hint">{t("table.select_a_slicer")}</div>
        </div>
      </div>
    );
  }
  const { slicer } = entry;

  const run = (action: () => void) => {
    try {
      action();
      setError("");
      onUpdate();
    } catch (e) {
      setError(messageOf(e));
    }
  };
  const apply = (patch: Partial<Slicer>) =>
    run(() => model.updateSlicer(slicer.name, { ...slicer, ...patch }));

  const columns = Math.max(slicer.button_columns, 1);
  const buttonWidth =
    (slicer.width - ITEMS_PADDING - ITEMS_GAP * (columns - 1)) / columns;

  // Distance of the slicer's corner from the top left of the sheet
  const horizontal = (() => {
    let total = slicer.offset_x;
    for (let c = 1; c < slicer.column; c += 1) {
      total += model.getColumnWidth(sheet, c);
    }
    return total;
  })();
  const vertical = (() => {
    let total = slicer.offset_y;
    for (let r = 1; r < slicer.row; r += 1) {
      total += model.getRowHeight(sheet, r);
    }
    return total;
  })();
  const moveTo = (x: number, y: number) => {
    let column = 1;
    let restX = x;
    for (
      let width = model.getColumnWidth(sheet, column);
      restX >= width;
      width = model.getColumnWidth(sheet, column)
    ) {
      restX -= width;
      column += 1;
    }
    let row = 1;
    let restY = y;
    for (
      let height = model.getRowHeight(sheet, row);
      restY >= height;
      height = model.getRowHeight(sheet, row)
    ) {
      restY -= height;
      row += 1;
    }
    apply({ row, column, offset_x: restX, offset_y: restY });
  };

  const textCommit = (
    event: FocusEvent<HTMLInputElement | HTMLTextAreaElement>,
    current: string,
    key: "caption" | "alt_text",
  ) => {
    if (event.target.value !== current) {
      apply({ [key]: event.target.value });
    }
  };

  return (
    <div className="ic-pivot-container">
      {header}
      <div className="ic-pivot-body">
        {error ? <div className="ic-pivot-error">{error}</div> : null}
        <div className="ic-pivot-label">{t("table.title")}</div>
        <Input
          key={`caption:${slicer.name}:${slicer.caption}`}
          type="text"
          size="sm"
          defaultValue={slicer.caption}
          aria-label={t("table.title")}
          onKeyDown={commitOnEnter}
          onBlur={(event) => textCommit(event, slicer.caption, "caption")}
        />
        <Checkbox
          label={t("table.show_header")}
          checked={slicer.show_header}
          onChange={(show_header) => apply({ show_header })}
        />
        <div className="ic-pivot-hint">
          {t("table.source")}: {entry.source_description}
        </div>
        <div className="ic-pivot-hint">
          {t("table.name")}: {slicer.name}
        </div>

        <div className="ic-pivot-section-title">
          {t("table.button_settings")}
        </div>
        <div className="ic-table-total-row">
          <span className="ic-table-total-name">
            {t("table.number_per_row")}
          </span>
          <Input
            key={`columns:${slicer.button_columns}`}
            type="text"
            inputMode="numeric"
            size="sm"
            defaultValue={String(slicer.button_columns)}
            aria-label={t("table.number_per_row")}
            onKeyDown={commitOnEnter}
            onBlur={(event) => {
              const value = Number.parseInt(event.target.value, 10);
              if (value >= 1 && value !== slicer.button_columns) {
                apply({ button_columns: value });
              }
            }}
          />
        </div>
        <MeasureField
          label={t("table.height")}
          value={slicer.button_height}
          onCommit={(button_height) => apply({ button_height })}
        />
        <MeasureField
          label={t("table.width")}
          value={buttonWidth}
          onCommit={(width) =>
            apply({
              width:
                width * columns + ITEMS_PADDING + ITEMS_GAP * (columns - 1),
            })
          }
        />

        <div className="ic-pivot-section-title">{t("table.slicer_size")}</div>
        <MeasureField
          label={t("table.height")}
          value={slicer.height}
          onCommit={(height) => apply({ height })}
        />
        <MeasureField
          label={t("table.width")}
          value={slicer.width}
          onCommit={(width) => apply({ width })}
        />

        <div className="ic-pivot-section-title">
          {t("table.slicer_position")}
        </div>
        <MeasureField
          label={t("table.horizontal")}
          value={horizontal}
          onCommit={(x) => moveTo(x, vertical)}
        />
        <MeasureField
          label={t("table.vertical")}
          value={vertical}
          onCommit={(y) => moveTo(horizontal, y)}
        />
        <Checkbox
          label={t("table.disable_moving")}
          checked={slicer.locked}
          onChange={(locked) => apply({ locked })}
        />
        <Select
          size="sm"
          value={slicer.placement}
          options={[
            { value: "move_and_size", label: t("table.move_and_size") },
            { value: "move", label: t("table.move_only") },
            { value: "free", label: t("table.free") },
          ]}
          onChange={(next) => apply({ placement: next as SlicerPlacement })}
        />

        <div className="ic-pivot-section-title">{t("table.properties")}</div>
        <Checkbox
          label={t("table.print_object")}
          checked={slicer.print}
          onChange={(print) => apply({ print })}
        />

        {slicer.source.type === "pivot" ? (
          <>
            <div className="ic-pivot-section-title">
              {t("table.pivot_connections")}
            </div>
            {entry.connections.map((connection) => (
              <Checkbox
                key={`${connection.sheet_id}:${connection.name}`}
                label={`${connection.name} (${connection.sheet_name})`}
                checked={connection.connected}
                onChange={(on) => {
                  const links = entry.connections
                    .filter((c) => (c === connection ? on : c.connected))
                    .map((c) => ({ name: c.name, sheet_id: c.sheet_id }));
                  run(() => model.setSlicerConnections(slicer.name, links));
                }}
              />
            ))}
          </>
        ) : null}

        <div className="ic-pivot-section-title">{t("table.alt_text")}</div>
        <textarea
          key={`alt:${slicer.name}:${slicer.alt_text}`}
          className="ic-table-textarea"
          rows={3}
          defaultValue={slicer.alt_text}
          aria-label={t("table.alt_text")}
          onKeyDown={(event) => event.stopPropagation()}
          onBlur={(event) => textCommit(event, slicer.alt_text, "alt_text")}
        />

        <div className="ic-pivot-section-title">{t("table.slicer_styles")}</div>
        <div className="ic-table-swatches">
          {STYLES.map((style) => {
            const { accent, solid } = slicerColors(model, style);
            const label = style.replace("SlicerStyle", "");
            return (
              <button
                type="button"
                key={style}
                className={`ic-slicer-swatch${
                  slicer.style === style ? " ic-table-swatch--selected" : ""
                }${solid ? " ic-slicer--solid" : ""}`}
                style={{ "--ic-slicer-accent": accent } as React.CSSProperties}
                aria-label={label}
                aria-pressed={slicer.style === style}
                title={label}
                onClick={() => apply({ style })}
              >
                <span className="ic-slicer-item ic-slicer-item--on" />
                <span className="ic-slicer-item ic-slicer-item--on" />
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
};

export default SlicerSettingsPanel;
