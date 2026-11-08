import type { Model } from "@ironcalc/workbook";
import { quoteSheetName } from "./references";

// A citation is stored as an internal cell link located at [Workbook]Sheet!B2.
// Links are saved with the file and follow their cell when rows or columns
// move, and the grid's own link tooltip shows the source on hover.
const LOCATION = /^\[([^\]]+)\]('(?:[^']|'')+'|[^!']+)!([A-Za-z]+\d+)$/;

export interface Citation {
  workbook: string;
  sheet: string;
  cell: string;
}

export function formatCitation(citation: Citation): string {
  const sheet = quoteSheetName(citation.sheet);
  return `[${citation.workbook}]${sheet}!${citation.cell}`;
}

export function parseCitation(location: string): Citation | null {
  const match = location.match(LOCATION);
  if (!match) {
    return null;
  }
  const sheet = match[2].startsWith("'")
    ? match[2].slice(1, -1).replace(/''/g, "'")
    : match[2];
  return { workbook: match[1], sheet, cell: match[3] };
}

export function readCitation(
  model: Model,
  sheet: number,
  row: number,
  column: number,
): Citation | null {
  const link = model.getCellLink(sheet, row, column);
  return link?.type === "Internal" ? parseCitation(link.location) : null;
}
