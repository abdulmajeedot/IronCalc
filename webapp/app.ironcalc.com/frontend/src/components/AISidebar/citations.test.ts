import type { Model } from "@ironcalc/workbook";
import { describe, expect, it } from "vitest";
import { formatCitation, parseCitation, readCitation } from "./citations";

describe("citations", () => {
  it("round-trips plain and quoted sheet names", () => {
    for (const citation of [
      { workbook: "North Region", sheet: "Sales", cell: "B2" },
      { workbook: "North", sheet: "Bob's Q1", cell: "AA10" },
    ]) {
      expect(parseCitation(formatCitation(citation))).toEqual(citation);
    }
  });

  it("writes the source like an Excel reference", () => {
    expect(
      formatCitation({ workbook: "North", sheet: "Q1 Sales", cell: "B2" }),
    ).toBe("[North]'Q1 Sales'!B2");
  });

  it("ignores ordinary links", () => {
    expect(parseCitation("Sheet1!A1")).toBeNull();
    expect(parseCitation("[North]Sales!A1:B2")).toBeNull();
    const external = {
      getCellLink: () => ({ type: "External", target: "https://example.com" }),
    } as unknown as Model;
    expect(readCitation(external, 0, 1, 1)).toBeNull();
    const internal = {
      getCellLink: () => ({ type: "Internal", location: "Sheet2!C3" }),
    } as unknown as Model;
    expect(readCitation(internal, 0, 1, 1)).toBeNull();
  });

  it("reads a citation from a cell", () => {
    const cited = {
      getCellLink: () => ({ type: "Internal", location: "[North]Sales!B2" }),
    } as unknown as Model;
    expect(readCitation(cited, 0, 1, 1)).toEqual({
      workbook: "North",
      sheet: "Sales",
      cell: "B2",
    });
  });
});
