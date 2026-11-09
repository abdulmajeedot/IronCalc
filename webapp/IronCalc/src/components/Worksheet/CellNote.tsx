import {
  type KeyboardEvent,
  type RefObject,
  type SyntheticEvent,
  useEffect,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import {
  headerColumnWidth,
  headerRowHeight,
} from "../WorksheetCanvas/constants";
import type WorksheetCanvas from "../WorksheetCanvas/worksheetCanvas";

const GAP = 8;

// Puts a note beside its cell, on the left if there is no room on the right.
// Called after every paint so the note follows the sheet as it scrolls.
export function placeCellNote(
  element: HTMLElement | null,
  canvas: WorksheetCanvas,
  row: number,
  column: number,
): void {
  const sheet = element?.parentElement;
  if (!element || !sheet) {
    return;
  }
  const cell = canvas.getCellRect(row, column);
  const offSheet =
    cell.x + cell.width <= headerColumnWidth ||
    cell.y + cell.height <= headerRowHeight ||
    cell.x >= sheet.clientWidth ||
    cell.y >= sheet.clientHeight;
  if (offSheet) {
    element.style.visibility = "hidden";
    return;
  }
  const { offsetWidth: width, offsetHeight: height } = element;
  let left = cell.x + cell.width + GAP;
  if (left + width > sheet.clientWidth) {
    left = cell.x - width - GAP;
  }
  const top = Math.min(cell.y, sheet.clientHeight - height - GAP);
  element.style.left = `${Math.max(left, headerColumnWidth + 2)}px`;
  element.style.top = `${Math.max(top, headerRowHeight + 2)}px`;
  element.style.visibility = "visible";
}

// The note shown while the pointer is over its cell. It ignores the pointer,
// so moving across it still reaches the cells underneath.
export const CellNotePopup = (props: {
  elementRef: RefObject<HTMLDivElement | null>;
  text: string;
  author: string;
}) => (
  <div
    ref={props.elementRef}
    className="ic-worksheet-note ic-worksheet-note-popup"
  >
    {props.author ? (
      <div className="ic-worksheet-note-author">{props.author}:</div>
    ) : null}
    {props.text}
  </div>
);

const stop = (event: SyntheticEvent): void => event.stopPropagation();

// Saves when it loses focus, like a note in Excel. Escape discards.
export const CellNoteEditor = (props: {
  elementRef: RefObject<HTMLDivElement | null>;
  initialText: string;
  onSave: (text: string) => void;
  onCancel: () => void;
}) => {
  const { elementRef, initialText, onSave, onCancel } = props;
  const { t } = useTranslation();
  const [text, setText] = useState(initialText);
  const textarea = useRef<HTMLTextAreaElement>(null);
  const finished = useRef(false);
  // What the listener below saves: it outlives any one render
  const save = useRef(() => {});
  save.current = () => onSave(text);

  useEffect(() => {
    const element = textarea.current;
    if (element) {
      element.focus({ preventScroll: true });
      element.setSelectionRange(initialText.length, initialText.length);
    }
  }, [initialText]);

  const finish = (action: () => void): void => {
    if (!finished.current) {
      finished.current = true;
      action();
    }
  };

  // A click elsewhere doesn't always take focus from the textarea (a toolbar
  // button keeps it, for one), so clicks are watched for directly.
  // biome-ignore lint/correctness/useExhaustiveDependencies: Only on mount; `finished` and `save` are refs
  useEffect(() => {
    const onPointerDown = (event: PointerEvent): void => {
      if (!elementRef.current?.contains(event.target as Node)) {
        if (!finished.current) {
          finished.current = true;
          save.current();
        }
      }
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    return () =>
      document.removeEventListener("pointerdown", onPointerDown, true);
  }, []);

  const onKeyDown = (event: KeyboardEvent): void => {
    event.stopPropagation();
    if (event.key === "Escape") {
      event.preventDefault();
      finish(onCancel);
    } else if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      finish(() => onSave(text));
    }
  };

  return (
    <div
      ref={elementRef}
      className="ic-worksheet-note ic-worksheet-note-editor"
      onPointerDown={stop}
    >
      <textarea
        ref={textarea}
        aria-label={t("note.label")}
        placeholder={t("note.placeholder")}
        value={text}
        onChange={(event) => setText(event.target.value)}
        onBlur={() => finish(() => onSave(text))}
        onKeyDown={onKeyDown}
        onDoubleClick={stop}
        onContextMenu={stop}
        onCopy={stop}
        onCut={stop}
        onPaste={stop}
      />
    </div>
  );
};
