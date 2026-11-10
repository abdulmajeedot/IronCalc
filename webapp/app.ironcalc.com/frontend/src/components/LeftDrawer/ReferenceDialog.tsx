import { DialogShell, Input } from "@ironcalc/workbook";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  type BackendDocument,
  type DocumentSnapshot,
  readDocumentSnapshot,
} from "../documents";
import { columnName, isValidRange, selectionRange } from "../linkedWorkbooks";

// More than this and the preview stops; the range box still reaches any cell.
const MAX_PREVIEW_ROWS = 200;
const MAX_PREVIEW_COLUMNS = 40;
const MIN_PREVIEW_ROWS = 20;
const MIN_PREVIEW_COLUMNS = 8;

type Cell = { row: number; column: number };

function shownValue(value: unknown): string {
  if (typeof value === "boolean") {
    return value ? "TRUE" : "FALSE";
  }
  if (value !== null && typeof value === "object" && "error" in value) {
    return String(value.error);
  }
  return String(value);
}

export default function ReferenceDialog(props: {
  document: BackendDocument;
  onClose: () => void;
  onInsert: (
    snapshot: DocumentSnapshot,
    sheet: string,
    range: string,
  ) => string | null;
}) {
  const { document, onClose, onInsert } = props;
  const { t } = useTranslation();
  const [snapshot, setSnapshot] = useState<DocumentSnapshot | null>(null);
  const [sheetIndex, setSheetIndex] = useState(0);
  const [anchor, setAnchor] = useState<Cell>({ row: 1, column: 1 });
  const [end, setEnd] = useState<Cell>({ row: 1, column: 1 });
  const [range, setRange] = useState("$A$1");
  const [error, setError] = useState("");

  useEffect(() => {
    let current = true;
    readDocumentSnapshot(document.id)
      .then((loaded) => {
        if (current) {
          setSnapshot(loaded);
        }
      })
      .catch(() => {
        if (current) {
          setError(t("left_drawer.backend.unreachable"));
        }
      });
    return () => {
      current = false;
    };
  }, [document.id, t]);

  const sheet = snapshot?.sheets[sheetIndex];
  const { values, rows, columns } = useMemo(() => {
    const found = new Map<string, unknown>();
    let lastRow = MIN_PREVIEW_ROWS;
    let lastColumn = MIN_PREVIEW_COLUMNS;
    for (const entry of sheet?.cells ?? []) {
      const [row, column, value] = entry as [number, number, unknown];
      found.set(`${row}:${column}`, value);
      lastRow = Math.max(lastRow, row);
      lastColumn = Math.max(lastColumn, column);
    }
    return {
      values: found,
      rows: Math.min(lastRow, MAX_PREVIEW_ROWS),
      columns: Math.min(lastColumn, MAX_PREVIEW_COLUMNS),
    };
  }, [sheet]);

  const pick = (cell: Cell, extend: boolean) => {
    const start = extend ? anchor : cell;
    setAnchor(start);
    setEnd(cell);
    setRange(selectionRange(start, cell));
    setError("");
  };

  const inSelection = (row: number, column: number) =>
    row >= Math.min(anchor.row, end.row) &&
    row <= Math.max(anchor.row, end.row) &&
    column >= Math.min(anchor.column, end.column) &&
    column <= Math.max(anchor.column, end.column);

  return (
    <DialogShell
      title={t("left_drawer.backend.reference_title", { name: document.name })}
      onClose={onClose}
      confirmLabel={t("left_drawer.backend.reference_insert")}
      confirmDisabled={!snapshot || !sheet || !isValidRange(range)}
      onConfirm={() => {
        if (snapshot && sheet) {
          const failure = onInsert(snapshot, sheet.name, range);
          if (failure) {
            setError(failure);
          }
        }
      }}
    >
      <div className="app-ic-reference">
        <div className="app-ic-reference-controls">
          <select
            aria-label={t("left_drawer.backend.reference_sheet")}
            value={sheetIndex}
            onChange={(event) => {
              setSheetIndex(Number(event.target.value));
              pick({ row: 1, column: 1 }, false);
            }}
          >
            {(snapshot?.sheets ?? []).map((entry, index) => (
              <option key={entry.name} value={index}>
                {entry.name}
              </option>
            ))}
          </select>
          <Input
            aria-label={t("left_drawer.backend.reference_range")}
            value={range}
            error={!isValidRange(range)}
            onChange={(event) => {
              setRange(event.target.value);
              setError("");
            }}
          />
        </div>
        <div className="app-ic-reference-grid">
          <table>
            <thead>
              <tr>
                <th />
                {Array.from({ length: columns }, (_, index) => (
                  // biome-ignore lint/suspicious/noArrayIndexKey: columns don't move
                  <th key={index}>{columnName(index + 1)}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {Array.from({ length: rows }, (_, rowIndex) => {
                const row = rowIndex + 1;
                return (
                  <tr key={row}>
                    <th>{row}</th>
                    {Array.from({ length: columns }, (_, columnIndex) => {
                      const column = columnIndex + 1;
                      const value = values.get(`${row}:${column}`);
                      return (
                        <td
                          key={column}
                          className={
                            inSelection(row, column) ? "is-selected" : undefined
                          }
                          onMouseDown={(event) => {
                            event.preventDefault();
                            pick({ row, column }, event.shiftKey);
                          }}
                        >
                          {value === undefined ? "" : shownValue(value)}
                        </td>
                      );
                    })}
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
        <p className="app-ic-reference-hint">
          {t("left_drawer.backend.reference_hint")}
        </p>
        {error ? <div className="app-ic-backend-error">{error}</div> : null}
      </div>
    </DialogShell>
  );
}
