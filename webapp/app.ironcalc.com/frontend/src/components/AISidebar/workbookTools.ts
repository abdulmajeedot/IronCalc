import type { AgentTool } from "@earendil-works/pi-agent-core";
import type { Model } from "@ironcalc/workbook";
import { type Citation, formatCitation, readCitation } from "./citations";
import {
  findReferences,
  type PromptReference,
  quoteSheetName,
} from "./references";

const LAST_ROW = 1_048_576;
const LAST_COLUMN = 16_384;
// Keeps one tool call from flooding the model's context or freezing the tab.
const MAX_CELLS = 5_000;
// There is no "used range" API, so the summary only looks this far right.
const USED_RANGE_COLUMNS = 52;
// Longer references are cut here; the model can read the rest with a tool.
const MAX_REFERENCE_LINES = 300;
const MAX_LISTED_WORKBOOKS = 30;
const MAX_PLAN_STEPS = 12;
// The finance convention for numbers brought in from another source.
const CITED_COLOR = "#00803C";

export interface CellRange {
  row: number;
  column: number;
  width: number;
  height: number;
}

export type WorkbookMode = "read" | "write";

export type WorkbookRunner = <T>(
  name: string | undefined,
  mode: WorkbookMode,
  work: (model: Model) => T,
) => Promise<T>;

export interface WorkbookInfo {
  name: string;
  isOpen: boolean;
}

export interface WorkbookAccess {
  /** Runs `work` on the named workbook, or the open one if none is named. */
  run: WorkbookRunner;
  list: () => WorkbookInfo[];
  /** Creates an empty workbook and returns its final name. */
  create: (name: string) => Promise<string>;
}

export interface PlanStep {
  text: string;
  done: boolean;
}

function columnNumber(name: string): number {
  let result = 0;
  for (const letter of name.toUpperCase()) {
    result = result * 26 + letter.charCodeAt(0) - 64;
  }
  return result;
}

function columnName(column: number): string {
  let name = "";
  let rest = column;
  while (rest > 0) {
    const digit = (rest - 1) % 26;
    name = String.fromCharCode(65 + digit) + name;
    rest = (rest - digit - 1) / 26;
  }
  return name;
}

function assertInSheet(range: CellRange, text: string) {
  if (
    range.row < 1 ||
    range.column < 1 ||
    range.row + range.height - 1 > LAST_ROW ||
    range.column + range.width - 1 > LAST_COLUMN
  ) {
    throw new Error(`"${text}" is outside the sheet.`);
  }
}

function assertSize(range: CellRange) {
  if (range.width * range.height > MAX_CELLS) {
    throw new Error(`That range is too large. Use at most ${MAX_CELLS} cells.`);
  }
}

const CELLS_PATTERN =
  /^\$?([A-Za-z]{1,3})\$?(\d{1,7})(?::\$?([A-Za-z]{1,3})\$?(\d{1,7}))?$/;
const COLUMNS_PATTERN = /^\$?([A-Za-z]{1,3}):\$?([A-Za-z]{1,3})$/;
const ROWS_PATTERN = /^\$?(\d{1,7}):\$?(\d{1,7})$/;

function isFullColumns(range: CellRange): boolean {
  return range.row === 1 && range.height === LAST_ROW;
}

function isFullRows(range: CellRange): boolean {
  return range.column === 1 && range.width === LAST_COLUMN;
}

function parseCorners(text: string): [number, number, number, number] | null {
  const columns = text.match(COLUMNS_PATTERN);
  if (columns) {
    return [1, columnNumber(columns[1]), LAST_ROW, columnNumber(columns[2])];
  }
  const rows = text.match(ROWS_PATTERN);
  if (rows) {
    return [Number(rows[1]), 1, Number(rows[2]), LAST_COLUMN];
  }
  const cells = text.match(CELLS_PATTERN);
  if (!cells) {
    return null;
  }
  const column = columnNumber(cells[1]);
  const row = Number(cells[2]);
  return [
    row,
    column,
    cells[4] ? Number(cells[4]) : row,
    cells[3] ? columnNumber(cells[3]) : column,
  ];
}

/** Parses "B2", "A1:C10", "D:F" or "3:5". The ends may be in any order. */
export function parseRange(text: string): CellRange {
  const corners = parseCorners(text.trim());
  if (!corners) {
    throw new Error(
      `"${text}" is not a range. Use a form like A1, A1:C10, D:D or 3:3.`,
    );
  }
  const [row1, column1, row2, column2] = corners;
  const range = {
    row: Math.min(row1, row2),
    column: Math.min(column1, column2),
    width: Math.abs(column2 - column1) + 1,
    height: Math.abs(row2 - row1) + 1,
  };
  assertInSheet(range, text);
  return range;
}

/** Whole columns and rows read like the grid's name box: "D:F" and "3:5". */
export function formatRange(range: CellRange): string {
  const lastColumn = columnName(range.column + range.width - 1);
  const lastRow = range.row + range.height - 1;
  if (isFullColumns(range)) {
    return `${columnName(range.column)}:${lastColumn}`;
  }
  if (isFullRows(range)) {
    return `${range.row}:${lastRow}`;
  }
  const start = `${columnName(range.column)}${range.row}`;
  if (range.width === 1 && range.height === 1) {
    return start;
  }
  return `${start}:${lastColumn}${lastRow}`;
}

/**
 * Encodes rows for `pasteCsvText`. Every field is quoted so tabs and newlines
 * survive and an empty one-column row isn't skipped as a blank line. Short
 * rows are padded because the engine silently drops rows of uneven length.
 */
export function toTabSeparated(rows: string[][], width: number): string {
  return rows
    .map((row) =>
      Array.from(
        { length: width },
        (_, index) => `"${(row[index] ?? "").replace(/"/g, '""')}"`,
      ).join("\t"),
    )
    .join("\n");
}

function boundsOfColumns(
  model: Model,
  sheet: number,
  from: number,
  to: number,
): CellRange | null {
  let firstRow = Number.POSITIVE_INFINITY;
  let lastRow = 0;
  let firstColumn = 0;
  let lastColumn = 0;
  for (let column = from; column <= to; column += 1) {
    const rows = model.getRowsWithData(sheet, column);
    if (rows.length === 0) {
      continue;
    }
    firstColumn ||= column;
    lastColumn = column;
    for (const row of rows) {
      firstRow = Math.min(firstRow, row);
      lastRow = Math.max(lastRow, row);
    }
  }
  if (lastRow === 0) {
    return null;
  }
  return {
    row: firstRow,
    column: firstColumn,
    width: lastColumn - firstColumn + 1,
    height: lastRow - firstRow + 1,
  };
}

function boundsOfRows(
  model: Model,
  sheet: number,
  range: CellRange,
): CellRange | null {
  assertSize({ ...range, width: 1 });
  let firstColumn = Number.POSITIVE_INFINITY;
  let lastColumn = 0;
  let firstRow = 0;
  let lastRow = 0;
  for (let row = range.row; row < range.row + range.height; row += 1) {
    const columns = model.getColumnsWithData(sheet, row);
    if (columns.length === 0) {
      continue;
    }
    firstRow ||= row;
    lastRow = row;
    for (const column of columns) {
      firstColumn = Math.min(firstColumn, column);
      lastColumn = Math.max(lastColumn, column);
    }
  }
  if (lastRow === 0) {
    return null;
  }
  return {
    row: firstRow,
    column: firstColumn,
    width: lastColumn - firstColumn + 1,
    height: lastRow - firstRow + 1,
  };
}

/**
 * Shrinks whole columns or rows to the cells that hold data, so a selection of
 * a million mostly empty cells stays cheap. Null when there is no data.
 */
function dataArea(
  model: Model,
  sheet: number,
  range: CellRange,
): CellRange | null {
  if (isFullColumns(range)) {
    return boundsOfColumns(
      model,
      sheet,
      range.column,
      range.column + range.width - 1,
    );
  }
  return isFullRows(range) ? boundsOfRows(model, sheet, range) : range;
}

function usedRange(model: Model, sheet: number): CellRange | null {
  return boundsOfColumns(model, sheet, 1, USED_RANGE_COLUMNS);
}

export function selectedRange(model: Model): string {
  const [row1, column1, row2, column2] = model.getSelectedView().range;
  return formatRange({
    row: Math.min(row1, row2),
    column: Math.min(column1, column2),
    width: Math.abs(column2 - column1) + 1,
    height: Math.abs(row2 - row1) + 1,
  });
}

/** A short summary of the workbook, sent with each request. */
export function describeWorkbook(
  model: Model,
  otherWorkbooks: string[] = [],
): string {
  const sheets = model.getWorksheetsProperties();
  const sheet = model.getSelectedSheet();
  const used = usedRange(model, sheet);
  const visible = sheets.filter((item) => item.state === "visible");
  return [
    `Active sheet: ${sheets[sheet]?.name}.`,
    `All sheets: ${visible.map((item) => item.name).join(", ")}.`,
    `Selected: ${selectedRange(model)}.`,
    used ? `Cells in use: ${formatRange(used)}.` : "The sheet is empty.",
    `Formula language: ${model.getLanguage()}.`,
    ...(otherWorkbooks.length > 0
      ? [
          `Other workbooks: ${otherWorkbooks.slice(0, MAX_LISTED_WORKBOOKS).join(", ")}.`,
        ]
      : []),
  ].join(" ");
}

function describeFile(model: Model): string {
  return model
    .getWorksheetsProperties()
    .map((sheet, index) => {
      const used = usedRange(model, index);
      return `Sheet ${sheet.name}: ${used ? `cells in use ${formatRange(used)}` : "empty"}`;
    })
    .join("\n");
}

interface Target {
  sheet: number;
  range: CellRange;
  /** How to show the range to the user. */
  label: string;
}

const SHEET_PREFIX = /^(?:'((?:[^']|'')+)'|([^!']+))!(.+)$/;

/** Resolves "A1:C3" on the active sheet, or "Sheet2!A1:C3" on a named one. */
function resolveTarget(model: Model, text: string): Target {
  const trimmed = text.trim().replace(/^@/, "");
  const match = trimmed.match(SHEET_PREFIX);
  if (!match) {
    const range = parseRange(trimmed);
    return {
      sheet: model.getSelectedSheet(),
      range,
      label: formatRange(range),
    };
  }
  const name = match[1] ? match[1].replace(/''/g, "'") : match[2];
  const sheets = model.getWorksheetsProperties();
  const sheet = sheets.findIndex(
    (item) => item.name.toLowerCase() === name.toLowerCase(),
  );
  if (sheet < 0) {
    throw new Error(
      `There is no sheet named "${name}". The sheets are: ${sheets
        .map((item) => item.name)
        .join(", ")}.`,
    );
  }
  const range = parseRange(match[3]);
  return {
    sheet,
    range,
    label: `${quoteSheetName(sheets[sheet].name)}!${formatRange(range)}`,
  };
}

function readCells(model: Model, sheet: number, range: CellRange): string[] {
  const lines: string[] = [];
  for (let row = range.row; row < range.row + range.height; row += 1) {
    for (
      let column = range.column;
      column < range.column + range.width;
      column += 1
    ) {
      const content = model.getCellContent(sheet, row, column);
      if (content === "") {
        continue;
      }
      const name = formatRange({ row, column, width: 1, height: 1 });
      const shown = model.getFormattedCellValue(sheet, row, column);
      const citation = readCitation(model, sheet, row, column);
      const source = citation ? ` (source: ${formatCitation(citation)})` : "";
      lines.push(
        content.startsWith("=")
          ? `${name}: ${content} (value: ${shown})`
          : `${name}: ${shown}${source}`,
      );
    }
  }
  return lines;
}

/** Selects a citation's source cell. False if its sheet no longer exists. */
export function selectCitedCell(model: Model, citation: Citation): boolean {
  const sheet = model
    .getWorksheetsProperties()
    .findIndex((item) => item.name === citation.sheet);
  if (sheet < 0) {
    return false;
  }
  const { row, column } = parseRange(citation.cell);
  model.setSelectedSheet(sheet);
  model.setSelectedCell(row, column);
  model.setTopLeftVisibleCell(Math.max(1, row - 2), Math.max(1, column - 1));
  return true;
}

interface PulledCells {
  rows: string[][];
  /** Each cell's source, or null for an empty cell. */
  sources: (string | null)[][];
  count: number;
}

function readForPull(model: Model, text: string): PulledCells {
  const target = resolveTarget(model, text);
  const area = dataArea(model, target.sheet, target.range);
  if (!area) {
    throw new Error(`${target.label} is empty.`);
  }
  assertSize(area);
  const workbook = model.getName();
  const sheet = model.getWorksheetsProperties()[target.sheet].name;
  const rows: string[][] = [];
  const sources: (string | null)[][] = [];
  let count = 0;
  for (let row = area.row; row < area.row + area.height; row += 1) {
    const rowValues: string[] = [];
    const rowSources: (string | null)[] = [];
    for (
      let column = area.column;
      column < area.column + area.width;
      column += 1
    ) {
      const content = model.getCellContent(target.sheet, row, column);
      if (content === "") {
        rowValues.push("");
        rowSources.push(null);
        continue;
      }
      const cell = formatRange({ row, column, width: 1, height: 1 });
      rowValues.push(
        content.startsWith("=")
          ? model.getFormattedCellValue(target.sheet, row, column)
          : content,
      );
      rowSources.push(formatCitation({ workbook, sheet, cell }));
      count += 1;
    }
    rows.push(rowValues);
    sources.push(rowSources);
  }
  return { rows, sources, count };
}

/** Pastes rows at a target's top-left cell and returns the area written. */
function pasteRows(
  model: Model,
  target: Target,
  rows: string[][],
  description: string,
): CellRange {
  const width = Math.max(0, ...rows.map((row) => row.length));
  if (width === 0) {
    throw new Error("There is nothing to write.");
  }
  const range = {
    row: target.range.row,
    column: target.range.column,
    width,
    height: rows.length,
  };
  assertInSheet(range, description);
  assertSize(range);
  // Paste ends by selecting the pasted area, and the engine fails at that
  // point, skipping the recalculation, unless the selected cell is inside it
  // on the sheet being shown.
  if (target.sheet !== model.getSelectedSheet()) {
    model.setSelectedSheet(target.sheet);
  }
  model.setSelectedCell(range.row, range.column);
  model.pasteCsvText(
    { sheet: target.sheet, ...range },
    toTabSeparated(rows, width),
  );
  return range;
}

function describeRange(model: Model, text: string): string {
  const target = resolveTarget(model, text);
  const area = dataArea(model, target.sheet, target.range);
  if (!area) {
    return "(empty)";
  }
  // A large reference still gives the model its start rather than nothing.
  const width = Math.min(area.width, MAX_CELLS);
  const height = Math.min(area.height, Math.floor(MAX_CELLS / width));
  const lines = readCells(model, target.sheet, { ...area, width, height });
  const isPartial =
    lines.length > MAX_REFERENCE_LINES ||
    width < area.width ||
    height < area.height;
  const shown = lines.slice(0, MAX_REFERENCE_LINES).join("\n");
  return isPartial
    ? `${shown}\n(Only part of this range is shown. Use read_range on a smaller range to see the rest.)`
    : shown || "(empty)";
}

async function describeReference(
  run: WorkbookRunner,
  reference: PromptReference,
): Promise<string> {
  try {
    return await run(reference.workbook, "read", (model) =>
      reference.range
        ? describeRange(model, `${reference.sheet}!${reference.range}`)
        : describeFile(model),
    );
  } catch (e) {
    return `(could not be read: ${e instanceof Error ? e.message : String(e)})`;
  }
}

/** The contents of everything mentioned in the prompt, or "" if nothing is. */
export async function describeReferences(
  run: WorkbookRunner,
  prompt: string,
): Promise<string> {
  const mentions = new Map(
    findReferences(prompt).map((item) => [item.text, item]),
  );
  const blocks = await Promise.all(
    [...mentions.values()].map(
      async (item) => `${item.text}\n${await describeReference(run, item)}`,
    ),
  );
  return blocks.join("\n\n");
}

function inWorkbook(workbook: string | undefined, label: string): string {
  return workbook?.trim() ? `[${workbook.trim()}]${label}` : label;
}

function done(text: string, label: string) {
  return {
    content: [{ type: "text" as const, text }],
    details: { range: label },
  };
}

const STYLE_PATHS: Record<string, string> = {
  bold: "font.b",
  italic: "font.i",
  font_color: "font.color",
  fill_color: "fill.color",
  number_format: "num_fmt",
};

const rangeParameter = {
  type: "string",
  description:
    "A cell or range like A1, A1:C10, a whole column like D:D, or a whole row like 3:3. Prefix a sheet name for another sheet, like Sheet2!A1:C10.",
};

const workbookParameter = {
  type: "string",
  description:
    "The name of another workbook, from list_workbooks. Leave out for the open workbook.",
};

// biome-ignore lint/suspicious/noExplicitAny: Tool arguments are checked against each tool's schema at runtime.
export function createWorkbookTools(access: WorkbookAccess): AgentTool<any>[] {
  return [
    {
      name: "list_workbooks",
      label: "List workbooks",
      description: "List the user's workbooks and say which one is open.",
      parameters: { type: "object", properties: {} },
      execute: async () =>
        done(
          access
            .list()
            .map((item) => `${item.name}${item.isOpen ? " (open)" : ""}`)
            .join("\n"),
          "",
        ),
    },
    {
      name: "create_workbook",
      label: "Create workbook",
      description: "Create a new empty workbook. It starts with one sheet.",
      parameters: {
        type: "object",
        properties: { name: { type: "string" } },
        required: ["name"],
      },
      execute: async (_id, params: { name: string }) => {
        const name = await access.create(params.name);
        return done(`Created the workbook ${name}.`, name);
      },
    },
    {
      name: "update_plan",
      label: "Update plan",
      description:
        "Show the steps of a multi-step job to the user. Send the whole list each time, marking finished steps as done.",
      parameters: {
        type: "object",
        properties: {
          steps: {
            type: "array",
            items: {
              type: "object",
              properties: {
                text: { type: "string" },
                done: { type: "boolean" },
              },
              required: ["text", "done"],
            },
          },
        },
        required: ["steps"],
      },
      execute: async (_id, params: { steps: PlanStep[] }) => ({
        content: [{ type: "text" as const, text: "Plan updated." }],
        details: { plan: params.steps.slice(0, MAX_PLAN_STEPS) },
      }),
    },
    {
      name: "read_range",
      label: "Read range",
      description:
        "Read the non-empty cells of a range, with their values and formulas.",
      parameters: {
        type: "object",
        properties: { range: rangeParameter, workbook: workbookParameter },
        required: ["range"],
      },
      execute: async (_id, params: { range: string; workbook?: string }) =>
        access.run(params.workbook, "read", (model) => {
          const target = resolveTarget(model, params.range);
          const label = inWorkbook(params.workbook, target.label);
          const area = dataArea(model, target.sheet, target.range);
          if (!area) {
            return done(`${label} is empty.`, label);
          }
          assertSize(area);
          const lines = readCells(model, target.sheet, area);
          return done(lines.join("\n") || `${label} is empty.`, label);
        }),
    },
    {
      name: "write_range",
      label: "Write range",
      description:
        "Write rows of values or formulas, replacing what is there. Formulas start with =.",
      parameters: {
        type: "object",
        properties: {
          start: {
            type: "string",
            description: "The top-left cell to write to, like A1 or Sheet2!A1.",
          },
          rows: {
            type: "array",
            description: "The rows to write. Each row is a list of cells.",
            items: {
              type: "array",
              items: { type: ["string", "number", "boolean", "null"] },
            },
          },
          workbook: workbookParameter,
        },
        required: ["start", "rows"],
      },
      execute: async (
        _id,
        params: { start: string; rows: unknown[][]; workbook?: string },
      ) =>
        access.run(params.workbook, "write", (model) => {
          const target = resolveTarget(model, params.start);
          const rows = params.rows.map((row) =>
            row.map((cell) => String(cell ?? "")),
          );
          const range = pasteRows(model, target, rows, params.start);
          const label = inWorkbook(
            params.workbook,
            target.label.replace(/[^!]*$/, formatRange(range)),
          );
          return done(`Wrote ${label}.`, label);
        }),
    },
    {
      name: "pull_cells",
      label: "Pull cells",
      description:
        "Copy the values of a range into another place, usually from another workbook, and record where each value came from. Use this instead of retyping numbers.",
      parameters: {
        type: "object",
        properties: {
          from_range: rangeParameter,
          from_workbook: {
            type: "string",
            description:
              "The workbook to copy from. Leave out for the open workbook.",
          },
          to_start: {
            type: "string",
            description: "The top-left cell to copy to, like B2 or Summary!B2.",
          },
          to_workbook: {
            type: "string",
            description:
              "The workbook to copy to. Leave out for the open workbook.",
          },
        },
        required: ["from_range", "to_start"],
      },
      execute: async (
        _id,
        params: {
          from_range: string;
          from_workbook?: string;
          to_start: string;
          to_workbook?: string;
        },
      ) => {
        const pulled = await access.run(params.from_workbook, "read", (model) =>
          readForPull(model, params.from_range),
        );
        return access.run(params.to_workbook, "write", (model) => {
          const target = resolveTarget(model, params.to_start);
          const range = pasteRows(model, target, pulled.rows, params.to_start);
          pulled.sources.forEach((row, rowOffset) => {
            row.forEach((location, columnOffset) => {
              if (location) {
                model.setCellLink(
                  target.sheet,
                  range.row + rowOffset,
                  range.column + columnOffset,
                  { type: "Internal", location },
                  undefined,
                );
              }
            });
          });
          // A new link is styled as blue and underlined.
          const area = { sheet: target.sheet, ...range };
          model.updateRangeStyle(area, "font.u", "false");
          model.updateRangeStyle(area, "font.color", CITED_COLOR);
          const label = inWorkbook(
            params.to_workbook,
            target.label.replace(/[^!]*$/, formatRange(range)),
          );
          return done(`Pulled ${pulled.count} cells into ${label}.`, label);
        });
      },
    },
    {
      name: "format_range",
      label: "Format range",
      description:
        'Change one style of a range. bold and italic take "true" or "false". Colors are like #FF0000. number_format is like #,##0.00.',
      parameters: {
        type: "object",
        properties: {
          range: rangeParameter,
          style: { type: "string", enum: Object.keys(STYLE_PATHS) },
          value: { type: "string" },
          workbook: workbookParameter,
        },
        required: ["range", "style", "value"],
      },
      execute: async (
        _id,
        params: {
          range: string;
          style: string;
          value: string;
          workbook?: string;
        },
      ) => {
        const stylePath = STYLE_PATHS[params.style];
        if (!stylePath) {
          throw new Error(`Unknown style "${params.style}".`);
        }
        return access.run(params.workbook, "write", (model) => {
          const { sheet, range, label } = resolveTarget(model, params.range);
          // The engine stores the style of whole columns and rows just once.
          if (!isFullColumns(range) && !isFullRows(range)) {
            assertSize(range);
          }
          model.updateRangeStyle({ sheet, ...range }, stylePath, params.value);
          const shown = inWorkbook(params.workbook, label);
          return done(`Formatted ${shown}.`, shown);
        });
      },
    },
    {
      name: "clear_range",
      label: "Clear range",
      description: "Delete the contents of a range and keep its formatting.",
      parameters: {
        type: "object",
        properties: { range: rangeParameter, workbook: workbookParameter },
        required: ["range"],
      },
      execute: async (_id, params: { range: string; workbook?: string }) =>
        access.run(params.workbook, "write", (model) => {
          const { sheet, range, label } = resolveTarget(model, params.range);
          const shown = inWorkbook(params.workbook, label);
          // The engine visits every cell it is given, even empty ones.
          const area = dataArea(model, sheet, range);
          if (!area) {
            return done(`${shown} is already empty.`, shown);
          }
          assertSize(area);
          model.rangeClearContents(
            sheet,
            area.row,
            area.column,
            area.row + area.height - 1,
            area.column + area.width - 1,
          );
          return done(`Cleared ${shown}.`, shown);
        }),
    },
  ];
}
