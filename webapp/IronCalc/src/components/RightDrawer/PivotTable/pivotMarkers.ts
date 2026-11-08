import type { PivotMarker } from "@ironcalc/wasm";
import {
  headerColumnWidth,
  headerRowHeight,
} from "../../WorksheetCanvas/constants";
import type WorksheetCanvas from "../../WorksheetCanvas/worksheetCanvas";

const MARKER_SIZE = 16;
const MARKER_INSET = 2;

const GLYPHS: Record<PivotMarker["kind"], string> = {
  filter: "▾",
  expand: "+",
  collapse: "−",
};

// Draws a pivot's filter arrows and +/- buttons over the canvas. The buttons
// are rebuilt after every paint, so they follow scrolling and resizing.
export function renderPivotMarkers(
  worksheet: WorksheetCanvas,
  onActivate: (marker: PivotMarker, anchor: DOMRect) => void,
): void {
  const parent = worksheet.canvas.parentElement;
  if (!parent) {
    return;
  }
  for (const old of parent.querySelectorAll(".ic-pivot-marker")) {
    old.remove();
  }
  const { model } = worksheet;
  const sheet = model.getSelectedSheet();
  const { width, height } = worksheet.canvas.getBoundingClientRect();
  for (const marker of model.getPivotMarkers(sheet)) {
    const [x, y] = worksheet.getCoordinatesByCell(marker.row, marker.column);
    const [nextX] = worksheet.getCoordinatesByCell(
      marker.row,
      marker.column + 1,
    );
    const [, nextY] = worksheet.getCoordinatesByCell(
      marker.row + 1,
      marker.column,
    );
    const cellWidth = nextX - x;
    const cellHeight = nextY - y;
    const left = x + cellWidth - MARKER_SIZE - MARKER_INSET;
    const top = y + (cellHeight - MARKER_SIZE) / 2;
    if (
      left < headerColumnWidth ||
      top < headerRowHeight ||
      left + MARKER_SIZE > width ||
      top + MARKER_SIZE > height
    ) {
      continue;
    }
    const button = document.createElement("button");
    button.type = "button";
    button.className = `ic-pivot-marker ic-pivot-marker--${marker.kind}${
      marker.filtered ? " ic-pivot-marker--filtered" : ""
    }`;
    button.style.left = `${left}px`;
    button.style.top = `${top}px`;
    button.textContent = GLYPHS[marker.kind];
    button.setAttribute(
      "aria-label",
      marker.kind === "filter" ? marker.field : (marker.item ?? marker.field),
    );
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
