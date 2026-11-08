const PLAIN_SHEET_NAME = /^[\p{L}\p{N}_.]+$/u;

const CELL = String.raw`\$?[A-Za-z]{1,3}\$?\d{1,7}`;
const COLUMN = String.raw`\$?[A-Za-z]{1,3}`;
const ROW = String.raw`\$?\d{1,7}`;
const RANGE = `${CELL}(?::${CELL})?|${COLUMN}:${COLUMN}|${ROW}:${ROW}`;
const SHEET = String.raw`'(?:[^']|'')+'|[\p{L}\p{N}_.]+`;
const NOT_A_WORD_AFTER = String.raw`(?![\p{L}\p{N}_])`;
// Written like @Sheet1!A1:C3, @Sheet1!D:D, @Sheet1!3:3 or @'Q1 Sales'!B2. Another
// workbook comes first in brackets, as in Excel: @[North]Sales!A1, or @[North]
// for the whole file. The lookbehind skips text that only looks like a mention,
// such as an email address.
const REFERENCE = new RegExp(
  String.raw`(?<![\p{L}\p{N}_@])@(?:\[([^\]\r\n]+)\](?:(${SHEET})!(${RANGE})${NOT_A_WORD_AFTER})?|(${SHEET})!(${RANGE})${NOT_A_WORD_AFTER})`,
  "gu",
);

export interface PromptReference {
  start: number;
  end: number;
  /** The whole mention, including the @. */
  text: string;
  /** Set when the mention names a workbook. */
  workbook?: string;
  /** The sheet as written, with any quotes. Absent for a whole-file mention. */
  sheet?: string;
  range?: string;
}

export function findReferences(text: string): PromptReference[] {
  return [...text.matchAll(REFERENCE)].map((match) => {
    const start = match.index ?? 0;
    return {
      start,
      end: start + match[0].length,
      text: match[0],
      workbook: match[1],
      sheet: match[2] ?? match[4],
      range: match[3] ?? match[5],
    };
  });
}

export function quoteSheetName(name: string): string {
  return PLAIN_SHEET_NAME.test(name) ? name : `'${name.replace(/'/g, "''")}'`;
}

export function formatReference(
  sheetName: string,
  range: string,
  workbook?: string,
): string {
  const prefix = workbook ? `[${workbook}]` : "";
  return `@${prefix}${quoteSheetName(sheetName)}!${range}`;
}

export function formatWorkbookMention(workbook: string): string {
  return `@[${workbook}]`;
}
