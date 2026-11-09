import type { Model, PivotArea } from "@ironcalc/wasm";
import { columnNameFromNumber } from "@ironcalc/wasm";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Checkbox } from "../Checkbox/Checkbox";
import { Alert } from "../Modal";
import { DialogShell } from "../RightDrawer/PivotTable/PivotMenus";
import { messageOf } from "../RightDrawer/Table/TableMenus";
import "./dataTools.css";

// One cell selected means the block of data around it, as in Excel.
function targetArea(
  model: Model,
  view: ReturnType<Model["getSelectedView"]>,
): PivotArea {
  const [a, b, c, d] = view.range;
  if (a === c && b === d) {
    return model.getCurrentRegion(view.sheet, a, b);
  }
  return {
    first_row: Math.min(a, c),
    first_column: Math.min(b, d),
    last_row: Math.max(a, c),
    last_column: Math.max(b, d),
  };
}

// A header row has text in it and no numbers.
function looksLikeHeaders(model: Model, sheet: number, area: PivotArea) {
  for (let c = area.first_column; c <= area.last_column; c += 1) {
    const text = model.getFormattedCellValue(sheet, area.first_row, c);
    if (text === "" || !Number.isNaN(Number(text))) {
      return false;
    }
  }
  return true;
}

export const RemoveDuplicatesDialog = (props: {
  model: Model;
  onClose: () => void;
  refresh: () => void;
}) => {
  const { model, onClose, refresh } = props;
  const { t } = useTranslation();
  const [origin] = useState(() => model.getSelectedView());
  const [area] = useState(() => targetArea(model, origin));
  const [hasHeaders, setHasHeaders] = useState(() =>
    looksLikeHeaders(model, origin.sheet, area),
  );
  const columns: number[] = [];
  for (let c = area.first_column; c <= area.last_column; c += 1) {
    columns.push(c);
  }
  const [chosen, setChosen] = useState<number[]>(columns);
  const [result, setResult] = useState("");
  const [error, setError] = useState("");

  const isEmpty =
    area.first_row === area.last_row &&
    area.first_column === area.last_column &&
    model.getFormattedCellValue(
      origin.sheet,
      area.first_row,
      area.first_column,
    ) === "";

  // What will be compared is what is selected, like Excel. The header row
  // drops out of the selection when there is one.
  // biome-ignore lint/correctness/useExhaustiveDependencies: follows the checkbox only
  useEffect(() => {
    if (isEmpty) {
      return;
    }
    const first = area.first_row + (hasHeaders ? 1 : 0);
    model.setSelectedCell(Math.min(first, area.last_row), area.first_column);
    model.setSelectedRange(
      Math.min(first, area.last_row),
      area.first_column,
      area.last_row,
      area.last_column,
    );
    refresh();
  }, [hasHeaders]);

  const cancel = (): void => {
    const [a, b, c, d] = origin.range;
    model.setSelectedCell(origin.row, origin.column);
    model.setSelectedRange(a, b, c, d);
    refresh();
    onClose();
  };

  if (isEmpty) {
    return (
      <Alert
        open
        title={t("remove_duplicates.title")}
        message={t("remove_duplicates.select_data")}
        onClose={onClose}
      />
    );
  }
  if (result !== "") {
    return (
      <Alert
        open
        title={t("remove_duplicates.title")}
        message={result}
        onClose={onClose}
      />
    );
  }

  const label = (column: number): string => {
    const name = t("remove_duplicates.column", {
      name: columnNameFromNumber(column),
    });
    if (!hasHeaders) {
      return name;
    }
    return (
      model.getFormattedCellValue(origin.sheet, area.first_row, column) || name
    );
  };

  const apply = (): void => {
    try {
      const [removed, remaining] = model.removeDuplicates(
        origin.sheet,
        area,
        new Int32Array(chosen),
        hasHeaders,
      );
      refresh();
      setResult(
        removed === 0
          ? t("remove_duplicates.none")
          : t("remove_duplicates.removed", { removed, remaining }),
      );
    } catch (e) {
      setError(messageOf(e));
    }
  };

  return (
    <DialogShell
      title={t("remove_duplicates.title")}
      onClose={cancel}
      onConfirm={apply}
      confirmDisabled={chosen.length === 0}
      confirmLabel={t("data_validation.apply")}
    >
      <div className="ic-pivot-hint">{t("remove_duplicates.hint")}</div>
      <div className="ic-dup-toggles">
        <Checkbox
          label={t("remove_duplicates.select_all")}
          checked={chosen.length === columns.length}
          onChange={(on) => setChosen(on ? columns : [])}
        />
        <Checkbox
          label={t("remove_duplicates.has_headers")}
          checked={hasHeaders}
          onChange={setHasHeaders}
        />
      </div>
      <div className="ic-dup-columns">
        {columns.map((column) => (
          <Checkbox
            key={column}
            label={label(column)}
            checked={chosen.includes(column)}
            onChange={(on) =>
              setChosen(
                on
                  ? [...chosen, column].sort((x, y) => x - y)
                  : chosen.filter((c) => c !== column),
              )
            }
          />
        ))}
      </div>
      {error ? <div className="ic-pivot-error">{error}</div> : null}
    </DialogShell>
  );
};
