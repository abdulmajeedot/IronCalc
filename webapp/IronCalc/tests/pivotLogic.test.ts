import type { PivotFieldInfo, PivotTable } from "@ironcalc/wasm";
import { describe, expect, it } from "vitest";
import {
  addField,
  currentRegion,
  displayOrder,
  moveFieldInZone,
  moveItem,
  moveWithin,
  nextSheetName,
  patchField,
  placeField,
  removeField,
  setAllCollapsed,
  toggleCollapsed,
  zoneOfField,
} from "../src/components/RightDrawer/PivotTable/pivotLogic";

const empty: PivotTable = {
  name: "PivotTable1",
  sheet_id: 1,
  row: 3,
  column: 1,
  source_sheet_id: 1,
  source: { first_row: 1, first_column: 1, last_row: 5, last_column: 3 },
  rows: [],
  columns: [],
  values: [],
  filters: [],
  options: {
    grand_totals_rows: true,
    grand_totals_columns: true,
    subtotals: "top",
    layout: "compact",
    repeat_labels: false,
  },
  output: null,
  style_name: "",
  read_only: false,
};

const text: PivotFieldInfo = { name: "Region", numeric: false, items: [] };
const number: PivotFieldInfo = { name: "Units", numeric: true, items: [] };

describe("pivot field placement", () => {
  it("sends text to rows and numbers to values", () => {
    const pivot = addField(addField(empty, text), number);
    expect(pivot.rows.map((f) => f.field)).toEqual(["Region"]);
    expect(pivot.values).toEqual([
      {
        field: "Units",
        aggregation: "sum",
        name: "",
        show_as: "normal",
        base_field: "",
      },
    ]);
  });

  it("removes a field from every zone", () => {
    const pivot = placeField(
      addField(empty, text),
      "Region",
      false,
      "values",
      0,
    );
    expect(pivot.rows).toEqual([]);
    expect(removeField(pivot, "Region").values).toEqual([]);
  });

  it("keeps a field on one axis at a time", () => {
    const pivot = placeField(
      addField(empty, text),
      "Region",
      false,
      "columns",
      0,
    );
    expect(pivot.rows).toEqual([]);
    expect(pivot.columns.map((f) => f.field)).toEqual(["Region"]);
  });

  it("keeps hidden items when a field changes axis", () => {
    const region = addField(empty, text).rows[0];
    const start = {
      ...empty,
      rows: [{ ...region, hidden_items: ["East"] }],
    };
    const pivot = placeField(start, "Region", false, "filters", 0);
    expect(pivot.filters[0].hidden_items).toEqual(["East"]);
  });

  it("counts text dropped on values", () => {
    const pivot = placeField(empty, "Region", false, "values", 0);
    expect(pivot.values[0].aggregation).toBe("count");
  });

  it("reorders within a zone and stops at the ends", () => {
    const start = placeField(
      addField(empty, text),
      "Product",
      false,
      "rows",
      1,
    );
    const moved = moveWithin(start, "rows", 1, -1);
    expect(moved.rows.map((f) => f.field)).toEqual(["Product", "Region"]);
    expect(moveWithin(start, "rows", 0, -1)).toBe(start);
  });
});

describe("currentRegion", () => {
  const cells = new Set(["2,2", "2,3", "3,2", "4,4", "3,3"]);
  const has = (row: number, column: number) => cells.has(`${row},${column}`);

  it("grows to the connected block, diagonals included", () => {
    expect(currentRegion(has, 3, 3)).toEqual({
      first_row: 2,
      first_column: 2,
      last_row: 4,
      last_column: 4,
    });
  });

  it("stays on an isolated cell", () => {
    expect(currentRegion(has, 20, 20)).toEqual({
      first_row: 20,
      first_column: 20,
      last_row: 20,
      last_column: 20,
    });
  });
});

describe("nextSheetName", () => {
  it("skips names in use, ignoring case", () => {
    expect(nextSheetName(["pivot1", "Pivot2"], "Pivot")).toBe("Pivot3");
  });
});

describe("right-click actions", () => {
  const withRegion = addField(empty, text);
  const items = ["East", "North", "West"];

  it("finds where a field is placed", () => {
    expect(zoneOfField(withRegion, "Region")).toBe("rows");
    expect(zoneOfField(withRegion, "Other")).toBe(null);
  });

  it("lists items in the order the pivot shows them", () => {
    const descending = patchField(withRegion, "Region", {
      sort: { order: "descending", by_value: null },
    });
    expect(displayOrder(descending, "Region", items)).toEqual([
      "West",
      "North",
      "East",
    ]);
  });

  it("moving an item fixes the order by hand", () => {
    const moved = moveItem(withRegion, "Region", "West", "first", items);
    expect(moved.rows[0].sort.order).toBe("manual");
    expect(moved.rows[0].item_order).toEqual(["West", "East", "North"]);
    const down = moveItem(withRegion, "Region", "East", "down", items);
    expect(down.rows[0].item_order).toEqual(["North", "East", "West"]);
  });

  it("toggles one item and collapses a whole field", () => {
    const collapsed = toggleCollapsed(withRegion, "Region", "East");
    expect(collapsed.rows[0].collapsed_items).toEqual(["East"]);
    expect(
      toggleCollapsed(collapsed, "Region", "East").rows[0].collapsed_items,
    ).toEqual([]);
    expect(
      setAllCollapsed(withRegion, "Region", items, true).rows[0]
        .collapsed_items,
    ).toEqual(items);
  });

  it("moves a field within its zone", () => {
    const two = placeField(withRegion, "Product", false, "rows", 1);
    const moved = moveFieldInZone(two, "rows", 1, "first");
    expect(moved.rows.map((f) => f.field)).toEqual(["Product", "Region"]);
    expect(moveFieldInZone(two, "rows", 0, -1)).toBe(two);
  });
});
