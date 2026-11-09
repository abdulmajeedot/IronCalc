import type {
  Model,
  ValidationAlertStyle,
  ValidationKind,
  ValidationOperator,
  ValidationSettings,
} from "@ironcalc/wasm";
import { columnNameFromNumber } from "@ironcalc/wasm";
import { Grid3x3 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../Button/Button";
import { IconButton } from "../Button/IconButton";
import { Checkbox } from "../Checkbox/Checkbox";
import { Input } from "../Input/Input";
import { DialogShell } from "../RightDrawer/PivotTable/PivotMenus";
import { messageOf } from "../RightDrawer/Table/TableMenus";
import { Select } from "../Select/Select";
import "./dataTools.css";

const KINDS: ValidationKind[] = [
  "none",
  "whole",
  "decimal",
  "list",
  "date",
  "time",
  "text_length",
  "custom",
];
const OPERATORS: ValidationOperator[] = [
  "between",
  "not_between",
  "equal",
  "not_equal",
  "greater_than",
  "less_than",
  "greater_than_or_equal",
  "less_than_or_equal",
];
const STYLES: ValidationAlertStyle[] = ["stop", "warning", "information"];
const TITLE_LIMIT = 32;
const MESSAGE_LIMIT = 255;

const EMPTY: ValidationSettings = {
  kind: "none",
  operator: "between",
  formula1: "",
  formula2: "",
  ignore_blank: true,
  in_cell_dropdown: true,
  show_input: true,
  input_title: "",
  input_message: "",
  show_error: true,
  error_style: "stop",
  error_title: "",
  error_message: "",
};

type Tab = "settings" | "input" | "alert";
type Field = "formula1" | "formula2";

function quotedSheet(name: string): string {
  return /^[A-Za-z_][A-Za-z0-9_.]*$/.test(name)
    ? name
    : `'${name.replace(/'/g, "''")}'`;
}

export const DataValidationDialog = (props: {
  model: Model;
  onClose: () => void;
  refresh: () => void;
}) => {
  const { model, onClose, refresh } = props;
  const { t } = useTranslation();
  const [origin] = useState(() => model.getSelectedView());
  const [form, setForm] = useState<ValidationSettings>(
    () =>
      model.getDataValidation(origin.sheet, origin.row, origin.column) ?? EMPTY,
  );
  const [tab, setTab] = useState<Tab>("settings");
  const [error, setError] = useState("");
  const [picking, setPicking] = useState<Field | null>(null);

  const update = (change: Partial<ValidationSettings>): void => {
    setForm((current) => ({ ...current, ...change }));
    setError("");
  };

  const selectOrigin = (): void => {
    const [a, b, c, d] = origin.range;
    model.setSelectedSheet(origin.sheet);
    model.setSelectedCell(origin.row, origin.column);
    model.setSelectedRange(a, b, c, d);
  };

  const finishPicking = (accept: boolean): void => {
    if (accept && picking) {
      const view = model.getSelectedView();
      const [r1, c1, r2, c2] = view.range;
      const cell = (row: number, column: number) =>
        `$${columnNameFromNumber(column)}$${row}`;
      const range =
        r1 === r2 && c1 === c2
          ? cell(r1, c1)
          : `${cell(r1, c1)}:${cell(r2, c2)}`;
      const prefix =
        view.sheet === origin.sheet
          ? ""
          : `${quotedSheet(model.getWorksheetsProperties()[view.sheet].name)}!`;
      update({ [picking]: `=${prefix}${range}` });
    }
    selectOrigin();
    setPicking(null);
    refresh();
  };

  if (picking) {
    const [r1, c1, r2, c2] = model.getSelectedView().range;
    return (
      <div className="ic-dv-picker">
        <span>{t("data_validation.pick_hint")}</span>
        <span className="ic-dv-picker-range">
          {columnNameFromNumber(c1)}
          {r1}:{columnNameFromNumber(c2)}
          {r2}
        </span>
        <Button
          size="sm"
          variant="secondary"
          onClick={() => finishPicking(false)}
        >
          {t("common.cancel")}
        </Button>
        <Button size="sm" onClick={() => finishPicking(true)}>
          {t("data_validation.done")}
        </Button>
      </div>
    );
  }

  const { kind, operator } = form;
  const hasOperator = kind !== "none" && kind !== "list" && kind !== "custom";
  const twoValues =
    hasOperator && (operator === "between" || operator === "not_between");
  const incomplete =
    (kind !== "none" && form.formula1.trim() === "") ||
    (twoValues && form.formula2.trim() === "");

  const firstLabel = (): string => {
    if (kind === "list") {
      return t("data_validation.source");
    }
    if (kind === "custom") {
      return t("data_validation.formula");
    }
    if (operator === "equal" || operator === "not_equal") {
      return t("data_validation.value");
    }
    const isMaximum =
      operator === "less_than" || operator === "less_than_or_equal";
    if (kind === "date") {
      return t(`data_validation.${isMaximum ? "end_date" : "start_date"}`);
    }
    if (kind === "time") {
      return t(`data_validation.${isMaximum ? "end_time" : "start_time"}`);
    }
    return t(`data_validation.${isMaximum ? "maximum" : "minimum"}`);
  };
  const secondLabel = (): string => {
    if (kind === "date") {
      return t("data_validation.end_date");
    }
    if (kind === "time") {
      return t("data_validation.end_time");
    }
    return t("data_validation.maximum");
  };

  const valueField = (field: Field, label: string) => (
    <div>
      <div className="ic-pivot-label">{label}</div>
      <div className="ic-dv-field">
        <Input
          type="text"
          size="sm"
          value={form[field]}
          aria-label={label}
          onChange={(event) => update({ [field]: event.target.value })}
        />
        <IconButton
          icon={<Grid3x3 />}
          aria-label={t("data_validation.pick_range")}
          onClick={() => setPicking(field)}
        />
      </div>
    </div>
  );

  const textArea = (
    label: string,
    value: string,
    onChange: (text: string) => void,
  ) => (
    <div>
      <div className="ic-pivot-label">{label}</div>
      <textarea
        className="ic-dv-textarea"
        aria-label={label}
        maxLength={MESSAGE_LIMIT}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </div>
  );

  const titleField = (
    label: string,
    value: string,
    onChange: (text: string) => void,
  ) => (
    <div>
      <div className="ic-pivot-label">{label}</div>
      <Input
        type="text"
        size="sm"
        aria-label={label}
        maxLength={TITLE_LIMIT}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </div>
  );

  const apply = (): void => {
    const area = {
      first_row: Math.min(origin.range[0], origin.range[2]),
      first_column: Math.min(origin.range[1], origin.range[3]),
      last_row: Math.max(origin.range[0], origin.range[2]),
      last_column: Math.max(origin.range[1], origin.range[3]),
    };
    try {
      model.setDataValidation(origin.sheet, area, form);
      refresh();
      onClose();
    } catch (e) {
      setError(messageOf(e));
    }
  };

  const tabButton = (id: Tab, label: string) => (
    <button
      type="button"
      className={`ic-dv-tab${tab === id ? " ic-dv-tab--active" : ""}`}
      onClick={() => setTab(id)}
    >
      {label}
    </button>
  );

  return (
    <DialogShell
      title={t("data_validation.title")}
      onClose={onClose}
      onConfirm={apply}
      confirmDisabled={incomplete}
      confirmLabel={t("data_validation.apply")}
      footerStart={
        <Button
          size="md"
          variant="secondary"
          onClick={() => {
            setForm(EMPTY);
            setError("");
          }}
        >
          {t("data_validation.clear_all")}
        </Button>
      }
    >
      <div className="ic-dv-body">
        <div className="ic-dv-tabs">
          {tabButton("settings", t("data_validation.settings"))}
          {tabButton("input", t("data_validation.input_message"))}
          {tabButton("alert", t("data_validation.error_alert"))}
        </div>
        {tab === "settings" ? (
          <>
            <div className="ic-pivot-label">{t("data_validation.allow")}</div>
            <Select
              size="sm"
              value={kind}
              options={KINDS.map((value) => ({
                value,
                label: t(`data_validation.kinds.${value}`),
              }))}
              onChange={(next) =>
                update({
                  kind: next as ValidationKind,
                  formula1: "",
                  formula2: "",
                })
              }
            />
            <div className="ic-pivot-label">{t("data_validation.data")}</div>
            <Select
              size="sm"
              value={operator}
              disabled={!hasOperator}
              options={OPERATORS.map((value) => ({
                value,
                label: t(`data_validation.operators.${value}`),
              }))}
              onChange={(next) =>
                update({ operator: next as ValidationOperator })
              }
            />
            {kind !== "none" ? valueField("formula1", firstLabel()) : null}
            {twoValues ? valueField("formula2", secondLabel()) : null}
            {kind === "list" ? (
              <div className="ic-pivot-hint">
                {t("data_validation.source_hint")}
              </div>
            ) : null}
            <Checkbox
              label={t("data_validation.ignore_blank")}
              checked={form.ignore_blank}
              disabled={kind === "none"}
              onChange={(ignore_blank) => update({ ignore_blank })}
            />
            {kind === "list" ? (
              <Checkbox
                label={t("data_validation.in_cell_dropdown")}
                checked={form.in_cell_dropdown}
                onChange={(in_cell_dropdown) => update({ in_cell_dropdown })}
              />
            ) : null}
          </>
        ) : null}
        {tab === "input" ? (
          <>
            <Checkbox
              label={t("data_validation.show_message")}
              checked={form.show_input}
              onChange={(show_input) => update({ show_input })}
            />
            {titleField(
              t("data_validation.message_title"),
              form.input_title,
              (input_title) => update({ input_title }),
            )}
            {textArea(
              t("data_validation.message"),
              form.input_message,
              (input_message) => update({ input_message }),
            )}
          </>
        ) : null}
        {tab === "alert" ? (
          <>
            <Checkbox
              label={t("data_validation.show_alert")}
              checked={form.show_error}
              onChange={(show_error) => update({ show_error })}
            />
            <div className="ic-pivot-label">
              {t("data_validation.alert_type")}
            </div>
            <div className="ic-dv-radios">
              {STYLES.map((style) => (
                <label key={style}>
                  <input
                    type="radio"
                    name="ic-dv-alert-style"
                    checked={form.error_style === style}
                    onChange={() => update({ error_style: style })}
                  />
                  {t(`data_validation.styles.${style}`)}
                </label>
              ))}
            </div>
            {titleField(
              t("data_validation.alert_title"),
              form.error_title,
              (error_title) => update({ error_title }),
            )}
            {textArea(
              t("data_validation.alert_message"),
              form.error_message,
              (error_message) => update({ error_message }),
            )}
          </>
        ) : null}
        {error ? <div className="ic-pivot-error">{error}</div> : null}
      </div>
    </DialogShell>
  );
};

// What is shown when a typed value breaks the cell's rule.
export const ValidationAlertDialog = (props: {
  style: ValidationAlertStyle;
  title: string;
  message: string;
  // Keep the value
  onKeep: () => void;
  // Take the value back and let the user type again
  onRetry: () => void;
  // Take the value back
  onCancel: () => void;
}) => {
  const { style, title, message, onKeep, onRetry, onCancel } = props;
  const { t } = useTranslation();
  const text = message || t("data_validation.default_error");
  const heading = title || t("data_validation.default_error_title");
  if (style === "stop") {
    return (
      <DialogShell
        title={heading}
        onClose={onCancel}
        onConfirm={onRetry}
        confirmLabel={t("data_validation.retry")}
      >
        <div className="ic-pivot-hint">{text}</div>
      </DialogShell>
    );
  }
  if (style === "warning") {
    return (
      <DialogShell
        title={heading}
        onClose={onCancel}
        onConfirm={onKeep}
        confirmLabel={t("data_validation.yes")}
        footerStart={
          <Button size="md" variant="secondary" onClick={onRetry}>
            {t("data_validation.no")}
          </Button>
        }
      >
        <div className="ic-pivot-hint">{text}</div>
        <div className="ic-pivot-hint">{t("data_validation.continue")}</div>
      </DialogShell>
    );
  }
  return (
    <DialogShell
      title={heading}
      onClose={onCancel}
      onConfirm={onKeep}
      confirmLabel={t("data_validation.ok")}
    >
      <div className="ic-pivot-hint">{text}</div>
    </DialogShell>
  );
};
