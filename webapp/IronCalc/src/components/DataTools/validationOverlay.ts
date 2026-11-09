import type { Model } from "@ironcalc/wasm";
import { headerRowHeight } from "../WorksheetCanvas/constants";
import type WorksheetCanvas from "../WorksheetCanvas/worksheetCanvas";
import "./dataTools.css";

const ARROW_SIZE = 18;

function hasDropdown(model: Model, view: ReturnType<Model["getSelectedView"]>) {
  const rule = model.getDataValidation(view.sheet, view.row, view.column);
  return rule?.kind === "list" && rule.in_cell_dropdown ? rule : null;
}

export function cellHasDropdown(model: Model): boolean {
  return hasDropdown(model, model.getSelectedView()) !== null;
}

// The dropdown arrow and the input message of the selected cell, drawn over
// the canvas. Rebuilt after every paint, so they follow scrolling.
export function renderValidationOverlay(
  worksheet: WorksheetCanvas,
  options: {
    canEdit: boolean;
    isEditing: boolean;
    onOpenList: () => void;
  },
): void {
  const parent = worksheet.canvas.parentElement;
  if (!parent) {
    return;
  }
  for (const old of parent.querySelectorAll(
    ".ic-validation-arrow, .ic-validation-prompt",
  )) {
    old.remove();
  }
  const { model } = worksheet;
  const view = model.getSelectedView();
  const rule = model.getDataValidation(view.sheet, view.row, view.column);
  if (!rule || options.isEditing) {
    return;
  }
  const cell = worksheet.getCellRect(view.row, view.column);
  const { width, height } = worksheet.canvas.getBoundingClientRect();
  if (cell.y + cell.height <= headerRowHeight || cell.y >= height) {
    return;
  }
  const right = cell.x + cell.width + 1;
  if (
    options.canEdit &&
    hasDropdown(model, view) &&
    right + ARROW_SIZE <= width
  ) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "ic-validation-arrow";
    button.setAttribute("aria-label", "Open list");
    button.textContent = "▾";
    button.style.left = `${right}px`;
    button.style.top = `${cell.y + cell.height - ARROW_SIZE}px`;
    button.onpointerdown = (event) => event.stopPropagation();
    button.onclick = (event) => {
      event.stopPropagation();
      options.onOpenList();
    };
    parent.appendChild(button);
  }
  if (rule.show_input && (rule.input_title || rule.input_message)) {
    const prompt = document.createElement("div");
    prompt.className = "ic-validation-prompt";
    if (rule.input_title) {
      const title = document.createElement("div");
      title.className = "ic-validation-prompt-title";
      title.textContent = rule.input_title;
      prompt.appendChild(title);
    }
    prompt.appendChild(document.createTextNode(rule.input_message));
    prompt.style.left = `${Math.max(cell.x + 8, 0)}px`;
    prompt.style.top = `${cell.y + cell.height + 6}px`;
    parent.appendChild(prompt);
  }
}
