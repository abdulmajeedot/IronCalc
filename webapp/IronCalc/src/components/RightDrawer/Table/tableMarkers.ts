import type { TableMarker } from "@ironcalc/wasm";
import {
  headerColumnWidth,
  headerRowHeight,
} from "../../WorksheetCanvas/constants";
import type WorksheetCanvas from "../../WorksheetCanvas/worksheetCanvas";

const MARKER_SIZE = 16;
const MARKER_INSET = 2;

function glyph(marker: TableMarker): string {
  if (marker.sort_descending === true) {
    return "↓";
  }
  if (marker.sort_descending === false) {
    return "↑";
  }
  return "▾";
}

// Draws a table's filter buttons over the canvas. Like the pivot's, they are
// rebuilt after every paint, so they follow scrolling and resizing.
export function renderTableMarkers(
  worksheet: WorksheetCanvas,
  onActivate: (marker: TableMarker, anchor: DOMRect) => void,
): void {
  const parent = worksheet.canvas.parentElement;
  if (!parent) {
    return;
  }
  for (const old of parent.querySelectorAll(".ic-table-marker")) {
    old.remove();
  }
  const { model } = worksheet;
  const sheet = model.getSelectedSheet();
  const { width, height } = worksheet.canvas.getBoundingClientRect();
  for (const marker of model.getTableMarkers(sheet)) {
    const [x, y] = worksheet.getCoordinatesByCell(marker.row, marker.col);
    const [nextX] = worksheet.getCoordinatesByCell(marker.row, marker.col + 1);
    const [, nextY] = worksheet.getCoordinatesByCell(
      marker.row + 1,
      marker.col,
    );
    const left = nextX - MARKER_SIZE - MARKER_INSET;
    const top = y + (nextY - y - MARKER_SIZE) / 2;
    if (
      nextX - x < MARKER_SIZE ||
      left < headerColumnWidth ||
      top < headerRowHeight ||
      left + MARKER_SIZE > width ||
      top + MARKER_SIZE > height
    ) {
      continue;
    }
    const button = document.createElement("button");
    button.type = "button";
    button.className = `ic-pivot-marker ic-table-marker${
      marker.filtered ? " ic-pivot-marker--filtered" : ""
    }`;
    button.style.left = `${left}px`;
    button.style.top = `${top}px`;
    button.textContent = glyph(marker);
    button.setAttribute("aria-label", marker.label);
    // The sheet must not treat a press on the button as selecting a cell
    button.addEventListener("pointerdown", (event) => event.stopPropagation());
    button.addEventListener("dblclick", (event) => event.stopPropagation());
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      onActivate(marker, button.getBoundingClientRect());
    });
    parent.appendChild(button);
  }
}
