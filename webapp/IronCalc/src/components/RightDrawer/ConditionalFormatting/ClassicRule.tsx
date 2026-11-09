import { getTokens, type IronCalcTheme } from "@ironcalc/wasm";
import { Check, SquareMousePointer } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../Button/Button";
import { IconButton } from "../../Button/IconButton";
import { Checkbox } from "../../Checkbox/Checkbox";
import { Input } from "../../Input/Input";
import { Select } from "../../Select/Select";
import { Tooltip } from "../../Tooltip/Tooltip";
import { selectionToValue } from "./cfRuleMapping";
import type { RuleData } from "./EditRule";
import FormatStylePicker, { type FormatStyle } from "./FormatStylePicker";
import { getRuleDescription } from "./ruleDescription";

const OPERATOR_TYPES_WITH_SECOND_DROPDOWN = [
  "cell_value",
  "text",
  "date",
  "top_bottom",
];
const AVERAGE_OPERATORS = ["above_average", "below_average"];

interface ClassicRuleProps {
  onSave: (data: RuleData) => void;
  onCancel: () => void;
  initialValues?: RuleData;
  getSelectedArea: () => string;
  applyTo: string;
  onApplyToChange: (val: string) => void;
  formatStyle: FormatStyle;
  onFormatStyleChange: (style: FormatStyle) => void;
  onDescriptionChange?: (description: string) => void;
  resolveValue?: (val: string) => string;
  currentTheme: IronCalcTheme;
  // This tab only makes rules of this one type
  lockedRuleType?: string;
}

const ClassicRule = ({
  onSave,
  onCancel,
  initialValues,
  getSelectedArea,
  applyTo,
  onApplyToChange,
  formatStyle,
  onFormatStyleChange,
  onDescriptionChange,
  resolveValue,
  currentTheme,
  lockedRuleType,
}: ClassicRuleProps) => {
  const { t } = useTranslation();
  // Values from a rule of another type don't carry over to this tab
  const initialType = initialValues?.ruleType;
  const initial =
    (lockedRuleType && initialType === lockedRuleType) ||
    (!lockedRuleType && initialType !== "top_bottom")
      ? initialValues
      : undefined;
  const [ruleType, setRuleType] = useState(
    lockedRuleType ?? initial?.ruleType ?? "cell_value",
  );
  const [ruleOperator, setRuleOperator] = useState(
    initial?.ruleOperator ??
      (lockedRuleType === "top_bottom" ? "top_items" : "between"),
  );
  const [ruleValue, setRuleValue] = useState(
    initial?.ruleValue ?? (lockedRuleType === "top_bottom" ? "10" : ""),
  );
  const [ruleValue2, setRuleValue2] = useState(initial?.ruleValue2 ?? "");
  const [stopIfTrue, setStopIfTrue] = useState(
    initialValues?.stopIfTrue ?? false,
  );

  const ruleTypeOptions = [
    {
      value: "cell_value",
      label: t("conditional_formatting.rule_type_cell_value"),
    },
    { value: "text", label: t("conditional_formatting.rule_type_text") },
    { value: "date", label: t("conditional_formatting.rule_type_date") },
    { value: "formula", label: t("conditional_formatting.rule_type_formula") },
    {
      value: "duplicate_values",
      label: t("conditional_formatting.rule_type_duplicate_values"),
    },
    {
      value: "unique_values",
      label: t("conditional_formatting.rule_type_unique_values"),
    },
    { value: "blanks", label: t("conditional_formatting.rule_type_blanks") },
    {
      value: "no_blanks",
      label: t("conditional_formatting.rule_type_no_blanks"),
    },
    { value: "errors", label: t("conditional_formatting.rule_type_errors") },
    {
      value: "no_errors",
      label: t("conditional_formatting.rule_type_no_errors"),
    },
  ];

  const operatorOptionsByType: Record<
    string,
    { value: string; label: string }[]
  > = {
    cell_value: [
      { value: "between", label: t("conditional_formatting.op_between") },
      {
        value: "not_between",
        label: t("conditional_formatting.op_not_between"),
      },
      { value: "less_than", label: t("conditional_formatting.op_less_than") },
      {
        value: "less_than_or_equal",
        label: t("conditional_formatting.op_less_than_or_equal"),
      },
      {
        value: "greater_than",
        label: t("conditional_formatting.op_greater_than"),
      },
      {
        value: "greater_than_or_equal",
        label: t("conditional_formatting.op_greater_than_or_equal"),
      },
      { value: "equals", label: t("conditional_formatting.op_equals") },
      {
        value: "does_not_equal",
        label: t("conditional_formatting.op_does_not_equal"),
      },
    ],
    text: [
      { value: "contains", label: t("conditional_formatting.op_contains") },
      {
        value: "does_not_contain",
        label: t("conditional_formatting.op_does_not_contain"),
      },
      {
        value: "begins_with",
        label: t("conditional_formatting.op_begins_with"),
      },
      { value: "ends_with", label: t("conditional_formatting.op_ends_with") },
      { value: "equals", label: t("conditional_formatting.op_equals") },
    ],
    top_bottom: [
      { value: "top_items", label: t("conditional_formatting.op_top_items") },
      {
        value: "top_percent",
        label: t("conditional_formatting.op_top_percent"),
      },
      {
        value: "bottom_items",
        label: t("conditional_formatting.op_bottom_items"),
      },
      {
        value: "bottom_percent",
        label: t("conditional_formatting.op_bottom_percent"),
      },
      {
        value: "above_average",
        label: t("conditional_formatting.op_above_average"),
      },
      {
        value: "below_average",
        label: t("conditional_formatting.op_below_average"),
      },
    ],
    date: [
      { value: "between", label: t("conditional_formatting.op_between") },
      {
        value: "not_between",
        label: t("conditional_formatting.op_not_between"),
      },
      { value: "yesterday", label: t("conditional_formatting.op_yesterday") },
      { value: "today", label: t("conditional_formatting.op_today") },
      { value: "tomorrow", label: t("conditional_formatting.op_tomorrow") },
      {
        value: "in_last_7_days",
        label: t("conditional_formatting.op_in_last_7_days"),
      },
      {
        value: "in_next_7_days",
        label: t("conditional_formatting.op_in_next_7_days"),
      },
      { value: "last_week", label: t("conditional_formatting.op_last_week") },
      { value: "this_week", label: t("conditional_formatting.op_this_week") },
      { value: "next_week", label: t("conditional_formatting.op_next_week") },
      { value: "last_month", label: t("conditional_formatting.op_last_month") },
      { value: "this_month", label: t("conditional_formatting.op_this_month") },
      { value: "next_month", label: t("conditional_formatting.op_next_month") },
      { value: "last_year", label: t("conditional_formatting.op_last_year") },
      { value: "this_year", label: t("conditional_formatting.op_this_year") },
      { value: "next_year", label: t("conditional_formatting.op_next_year") },
    ],
  };

  const showSecondDropdown =
    OPERATOR_TYPES_WITH_SECOND_DROPDOWN.includes(ruleType);
  const currentOperatorOptions = operatorOptionsByType[ruleType] ?? [];
  const showTwoInputs =
    showSecondDropdown && ["between", "not_between"].includes(ruleOperator);
  const showValueInput =
    showSecondDropdown &&
    !(
      ruleType === "date" && !["between", "not_between"].includes(ruleOperator)
    ) &&
    !AVERAGE_OPERATORS.includes(ruleOperator);
  const isTopBottom = ruleType === "top_bottom";
  const rankError =
    isTopBottom &&
    showValueInput &&
    !(Number.isInteger(Number(ruleValue)) && Number(ruleValue) > 0);
  const showFormulaInput = ruleType === "formula";

  const formulaError = (() => {
    if (!showFormulaInput || !ruleValue.trim()) {
      return null;
    }
    if (!ruleValue.trim().startsWith("=")) {
      return t("conditional_formatting.formula_error_must_start_with_equals");
    }
    if (
      getTokens(ruleValue.trim().slice(1)).some((t) => t.token === "Illegal")
    ) {
      return t("conditional_formatting.formula_error_invalid");
    }
    return null;
  })();

  useEffect(() => {
    onDescriptionChange?.(
      getRuleDescription({
        ruleType,
        ruleOperator,
        ruleValue,
        ruleValue2,
        resolveValue,
        t,
      }),
    );
  }, [
    ruleType,
    ruleOperator,
    ruleValue,
    ruleValue2,
    resolveValue,
    onDescriptionChange,
    t,
  ]);

  return (
    <>
      <div className="ic-edit-rule-content">
        <div className="ic-edit-rule-section">
          <div className="ic-edit-rule-section-title">
            {t("conditional_formatting.apply_to")}
          </div>
          <div className="ic-edit-rule-field-wrapper">
            <span className="ic-edit-rule-label">
              {t("conditional_formatting.apply_to_range")}
            </span>
            <Input
              type="text"
              placeholder={t("conditional_formatting.apply_to_placeholder")}
              value={applyTo}
              onChange={(e) => onApplyToChange(e.target.value)}
              endAdornment={
                <Tooltip title={t("conditional_formatting.use_selection")}>
                  <IconButton
                    size="sm"
                    variant="secondary"
                    icon={<SquareMousePointer />}
                    aria-label={t("conditional_formatting.use_selection")}
                    onClick={() => onApplyToChange(getSelectedArea())}
                    className="ic-edit-rule-range-button"
                  />
                </Tooltip>
              }
            />
          </div>
        </div>
        <div className="ic-edit-rule-section">
          <div className="ic-edit-rule-section-title">
            {t("conditional_formatting.format_rules")}
          </div>
          <div className="ic-edit-rule-condition">
            <span className="ic-edit-rule-label">
              {t("conditional_formatting.highlight_cells_if")}
            </span>
            <div className="ic-edit-rule-dropdowns-row">
              {lockedRuleType ? null : (
                <Select
                  value={ruleType}
                  options={ruleTypeOptions}
                  onChange={(next) => {
                    setRuleType(next);
                    setRuleOperator(
                      operatorOptionsByType[next]?.[0]?.value ?? "",
                    );
                  }}
                  hideCheck
                />
              )}
              {showSecondDropdown && (
                <Select
                  value={ruleOperator}
                  options={currentOperatorOptions}
                  onChange={setRuleOperator}
                  hideCheck
                />
              )}
            </div>
            {showFormulaInput && (
              <Input
                type="text"
                placeholder={t("conditional_formatting.formula_placeholder")}
                value={ruleValue}
                error={!!formulaError}
                helperText={formulaError ?? undefined}
                onChange={(e) => setRuleValue(e.target.value)}
              />
            )}
            {showValueInput &&
              (showTwoInputs ? (
                <div className="ic-edit-rule-values-row">
                  <Input
                    type="text"
                    placeholder={t(
                      "conditional_formatting.rule_value_placeholder",
                    )}
                    value={ruleValue}
                    onChange={(e) => setRuleValue(e.target.value)}
                  />
                  <Input
                    type="text"
                    placeholder={t(
                      "conditional_formatting.rule_value_placeholder",
                    )}
                    value={ruleValue2}
                    onChange={(e) => setRuleValue2(e.target.value)}
                  />
                </div>
              ) : (
                <Input
                  type="text"
                  placeholder={t(
                    "conditional_formatting.rule_value_placeholder",
                  )}
                  value={ruleValue}
                  onChange={(e) => setRuleValue(e.target.value)}
                  endAdornment={
                    <Tooltip title={t("conditional_formatting.use_selection")}>
                      <IconButton
                        size="sm"
                        variant="secondary"
                        icon={<SquareMousePointer />}
                        aria-label={t("conditional_formatting.use_selection")}
                        onClick={() =>
                          setRuleValue(selectionToValue(getSelectedArea()))
                        }
                        className="ic-edit-rule-range-button"
                      />
                    </Tooltip>
                  }
                />
              ))}
          </div>
        </div>
        <div className="ic-edit-rule-section">
          <div className="ic-edit-rule-section-title">
            {t("conditional_formatting.format_style")}
          </div>
          <FormatStylePicker
            value={formatStyle}
            onChange={onFormatStyleChange}
            currentTheme={currentTheme}
          />
        </div>
        <div className="ic-edit-rule-section">
          <Checkbox
            checked={stopIfTrue}
            onChange={setStopIfTrue}
            label={t("conditional_formatting.stop_if_true")}
          />
        </div>
      </div>
      <div className="ic-edit-rule-footer">
        <Button variant="secondary" onClick={onCancel}>
          {t("conditional_formatting.cancel")}
        </Button>
        <Button
          startIcon={<Check />}
          disabled={
            !applyTo.trim() ||
            (showFormulaInput && (!ruleValue.trim() || !!formulaError)) ||
            (showValueInput &&
              (!ruleValue.trim() || (showTwoInputs && !ruleValue2.trim()))) ||
            rankError
          }
          onClick={() =>
            onSave({
              applyTo,
              ruleType,
              ruleOperator,
              ruleValue,
              ruleValue2,
              formatStyle,
              stopIfTrue,
            })
          }
        >
          {t("conditional_formatting.apply")}
        </Button>
      </div>
    </>
  );
};

export default ClassicRule;
