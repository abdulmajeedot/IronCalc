import type { Model } from "@ironcalc/wasm";
import type { WorkbookState } from "../workbookState";

// Puts what the user typed into a cell. The value goes in first so a rule can
// look at it the way any formula would; a rejected value is taken back by the
// alert the workbook then shows.
export function enterCellInput(
  model: Model,
  workbookState: WorkbookState,
  sheet: number,
  row: number,
  column: number,
  text: string,
): void {
  model.setUserInput(sheet, row, column, text);
  workbookState.setCopiedCells(null);
  const failure = model.getDataValidationFailure(sheet, row, column);
  if (failure) {
    workbookState.setValidationAlert({ sheet, row, column, text, failure });
  }
}
