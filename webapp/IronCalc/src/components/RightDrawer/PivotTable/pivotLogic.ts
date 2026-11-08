import type {
  PivotArea,
  PivotField,
  PivotFieldInfo,
  PivotTable,
  PivotValue,
} from "@ironcalc/wasm";

export type PivotZone = "filters" | "rows" | "columns" | "values";

export const AXIS_ZONES: readonly PivotZone[] = ["filters", "rows", "columns"];
export const ALL_ZONES: readonly PivotZone[] = [...AXIS_ZONES, "values"];

export const AGGREGATIONS = [
  "sum",
  "count",
  "average",
  "max",
  "min",
  "product",
  "count_numbers",
  "std_dev",
  "std_dev_p",
  "var",
  "var_p",
] as const;

// The largest number of rows or columns a single-cell selection grows to.
const MAX_REGION = 100000;

const axisField = (field: string): PivotField => ({
  field,
  hidden_items: [],
  sort: { order: "ascending", by_value: null },
  item_order: [],
  collapsed_items: [],
  blank_line: false,
});

const newValue = (
  field: string,
  aggregation: PivotValue["aggregation"],
): PivotValue => ({
  field,
  aggregation,
  name: "",
  show_as: "normal",
  base_field: "",
});

export function isFieldUsed(pivot: PivotTable, field: string): boolean {
  return (
    pivot.rows.some((f) => f.field === field) ||
    pivot.columns.some((f) => f.field === field) ||
    pivot.filters.some((f) => f.field === field) ||
    pivot.values.some((v) => v.field === field)
  );
}

function withoutField(pivot: PivotTable, field: string): PivotTable {
  return {
    ...pivot,
    rows: pivot.rows.filter((f) => f.field !== field),
    columns: pivot.columns.filter((f) => f.field !== field),
    filters: pivot.filters.filter((f) => f.field !== field),
  };
}

// A number goes to Values and anything else to Rows.
export function addField(pivot: PivotTable, info: PivotFieldInfo): PivotTable {
  if (info.numeric) {
    return {
      ...pivot,
      values: [...pivot.values, newValue(info.name, "sum")],
    };
  }
  return { ...pivot, rows: [...pivot.rows, axisField(info.name)] };
}

export function removeField(pivot: PivotTable, field: string): PivotTable {
  return {
    ...withoutField(pivot, field),
    values: pivot.values.filter((v) => v.field !== field),
  };
}

export function removeAt(
  pivot: PivotTable,
  zone: PivotZone,
  index: number,
): PivotTable {
  if (zone === "values") {
    return { ...pivot, values: pivot.values.filter((_, i) => i !== index) };
  }
  return { ...pivot, [zone]: pivot[zone].filter((_, i) => i !== index) };
}

function insertAt<T>(list: T[], index: number, item: T): T[] {
  const next = [...list];
  next.splice(Math.max(0, Math.min(index, next.length)), 0, item);
  return next;
}

export function fieldNameAt(
  pivot: PivotTable,
  zone: PivotZone,
  index: number,
): string {
  return pivot[zone][index].field;
}

// Puts a field in a zone at `index`. A field lives on one axis at a time, so
// it leaves the others. A value that moves keeps its settings; a text field
// dropped on Values counts instead of summing.
export function placeField(
  pivot: PivotTable,
  field: string,
  numeric: boolean,
  zone: PivotZone,
  index: number,
): PivotTable {
  if (zone === "values") {
    if (pivot.values.some((v) => v.field === field)) {
      const value = pivot.values.find((v) => v.field === field);
      const others = pivot.values.filter((v) => v.field !== field);
      return value
        ? {
            ...withoutField(pivot, field),
            values: insertAt(others, index, value),
          }
        : pivot;
    }
    return {
      ...withoutField(pivot, field),
      values: insertAt(
        pivot.values,
        index,
        newValue(field, numeric ? "sum" : "count"),
      ),
    };
  }
  const existing = AXIS_ZONES.flatMap((z) =>
    pivot[z as "filters" | "rows" | "columns"].filter((f) => f.field === field),
  )[0];
  const cleared = withoutField(pivot, field);
  return {
    ...cleared,
    values: pivot.values.filter((v) => v.field !== field),
    [zone]: insertAt(cleared[zone], index, existing ?? axisField(field)),
  };
}

// Moves an entry within its own zone.
export function moveWithin(
  pivot: PivotTable,
  zone: PivotZone,
  index: number,
  delta: number,
): PivotTable {
  const list = [...pivot[zone]] as unknown[];
  const target = index + delta;
  if (target < 0 || target >= list.length) {
    return pivot;
  }
  const [item] = list.splice(index, 1);
  list.splice(target, 0, item);
  return { ...pivot, [zone]: list };
}

// Grows a cell to the block of data around it: every row and column touching
// the block that holds something, until an empty border is reached.
export function currentRegion(
  hasValue: (row: number, column: number) => boolean,
  row: number,
  column: number,
): PivotArea {
  const area: PivotArea = {
    first_row: row,
    first_column: column,
    last_row: row,
    last_column: column,
  };
  // The border is scanned one cell past each end so corners count too.
  const rowHasValue = (r: number) => {
    const first = Math.max(1, area.first_column - 1);
    for (let c = first; c <= area.last_column + 1; c += 1) {
      if (hasValue(r, c)) {
        return true;
      }
    }
    return false;
  };
  const columnHasValue = (c: number) => {
    const first = Math.max(1, area.first_row - 1);
    for (let r = first; r <= area.last_row + 1; r += 1) {
      if (hasValue(r, c)) {
        return true;
      }
    }
    return false;
  };
  let grew = true;
  let steps = 0;
  while (grew && steps < MAX_REGION) {
    grew = false;
    steps += 1;
    if (area.first_row > 1 && rowHasValue(area.first_row - 1)) {
      area.first_row -= 1;
      grew = true;
    }
    if (rowHasValue(area.last_row + 1)) {
      area.last_row += 1;
      grew = true;
    }
    if (area.first_column > 1 && columnHasValue(area.first_column - 1)) {
      area.first_column -= 1;
      grew = true;
    }
    if (columnHasValue(area.last_column + 1)) {
      area.last_column += 1;
      grew = true;
    }
  }
  return area;
}

// A name for a new sheet that no sheet uses yet.
export function nextSheetName(existing: string[], base: string): string {
  const taken = new Set(existing.map((name) => name.toLowerCase()));
  let number = 1;
  while (taken.has(`${base}${number}`.toLowerCase())) {
    number += 1;
  }
  return `${base}${number}`;
}

export type AxisZone = "filters" | "rows" | "columns";

export function zoneOfField(pivot: PivotTable, field: string): AxisZone | null {
  for (const zone of AXIS_ZONES as AxisZone[]) {
    if (pivot[zone].some((f) => f.field === field)) {
      return zone;
    }
  }
  return null;
}

// Changes a field wherever it is placed.
export function patchField(
  pivot: PivotTable,
  field: string,
  changes: Partial<PivotField>,
): PivotTable {
  const update = (list: PivotField[]) =>
    list.map((f) => (f.field === field ? { ...f, ...changes } : f));
  return {
    ...pivot,
    rows: update(pivot.rows),
    columns: update(pivot.columns),
    filters: update(pivot.filters),
  };
}

export function patchValue(
  pivot: PivotTable,
  index: number,
  changes: Partial<PivotValue>,
): PivotTable {
  return {
    ...pivot,
    values: pivot.values.map((v, i) =>
      i === index ? { ...v, ...changes } : v,
    ),
  };
}

// The items of a field in the order the pivot lists them.
export function displayOrder(
  pivot: PivotTable,
  field: string,
  items: string[],
): string[] {
  const zone = zoneOfField(pivot, field);
  const layout = zone ? pivot[zone].find((f) => f.field === field) : undefined;
  if (!layout) {
    return items;
  }
  if (layout.sort.order === "descending") {
    return [...items].reverse();
  }
  if (layout.sort.order === "manual") {
    const listed = layout.item_order.filter((i) => items.includes(i));
    return [...listed, ...items.filter((i) => !listed.includes(i))];
  }
  return items;
}

export type ItemMove = "first" | "up" | "down" | "last";

// Moving an item fixes the order by hand, which replaces any automatic sort.
export function moveItem(
  pivot: PivotTable,
  field: string,
  item: string,
  where: ItemMove,
  order: string[],
): PivotTable {
  const from = order.indexOf(item);
  if (from < 0) {
    return pivot;
  }
  const next = order.filter((i) => i !== item);
  const to = {
    first: 0,
    up: Math.max(from - 1, 0),
    down: Math.min(from + 1, next.length),
    last: next.length,
  }[where];
  next.splice(to, 0, item);
  return patchField(pivot, field, {
    sort: { order: "manual", by_value: null },
    item_order: next,
  });
}

export function moveFieldInZone(
  pivot: PivotTable,
  zone: AxisZone,
  index: number,
  delta: number | "first" | "last",
): PivotTable {
  const list = [...pivot[zone]];
  const target =
    delta === "first" ? 0 : delta === "last" ? list.length - 1 : index + delta;
  if (index < 0 || target < 0 || target >= list.length) {
    return pivot;
  }
  const [item] = list.splice(index, 1);
  list.splice(target, 0, item);
  return { ...pivot, [zone]: list };
}

export function toggleCollapsed(
  pivot: PivotTable,
  field: string,
  item: string,
): PivotTable {
  const zone = zoneOfField(pivot, field);
  const layout = zone ? pivot[zone].find((f) => f.field === field) : undefined;
  if (!layout) {
    return pivot;
  }
  const collapsed = layout.collapsed_items.includes(item)
    ? layout.collapsed_items.filter((i) => i !== item)
    : [...layout.collapsed_items, item];
  return patchField(pivot, field, { collapsed_items: collapsed });
}

export function setAllCollapsed(
  pivot: PivotTable,
  field: string,
  items: string[],
  collapsed: boolean,
): PivotTable {
  return patchField(pivot, field, {
    collapsed_items: collapsed ? items : [],
  });
}
