import { describe, expect, it } from "vitest";
import {
  findReferences,
  formatReference,
  formatWorkbookMention,
  quoteSheetName,
} from "./references";

describe("findReferences", () => {
  it("finds several references with their positions", () => {
    const text = "Compare @Sheet1!A1:B4 with @Sheet2!C3.";
    const found = findReferences(text);
    expect(found.map((item) => item.text)).toEqual([
      "@Sheet1!A1:B4",
      "@Sheet2!C3",
    ]);
    expect(text.slice(found[1].start, found[1].end)).toBe("@Sheet2!C3");
    expect(found[0]).toMatchObject({ sheet: "Sheet1", range: "A1:B4" });
  });

  it("understands quoted sheet names", () => {
    expect(findReferences("see @'Q1 ''24 Sales'!B2 now")[0]).toMatchObject({
      sheet: "'Q1 ''24 Sales'",
      range: "B2",
    });
  });

  it("finds whole columns and rows", () => {
    expect(findReferences("@Data!D:D and @Data!B:F.")).toMatchObject([
      { range: "D:D" },
      { range: "B:F" },
    ]);
    expect(findReferences("@Data!3:3 and @Data!$2:$9")).toMatchObject([
      { range: "3:3" },
      { range: "$2:$9" },
    ]);
  });

  it("finds another workbook's cells and whole-file mentions", () => {
    expect(
      findReferences("Compare @[North Region]Sales!A1:C7 with @[South], then"),
    ).toMatchObject([
      {
        text: "@[North Region]Sales!A1:C7",
        workbook: "North Region",
        sheet: "Sales",
        range: "A1:C7",
      },
      { text: "@[South]", workbook: "South" },
    ]);
    expect(findReferences("@[South]")[0].range).toBeUndefined();
  });

  it("round-trips a mention with a workbook", () => {
    const text = formatReference("Q1 Sales", "D:D", "North Region");
    expect(text).toBe("@[North Region]'Q1 Sales'!D:D");
    expect(findReferences(text)[0].text).toBe(text);
    expect(formatWorkbookMention("North")).toBe("@[North]");
  });

  it("does not stop halfway through a longer word", () => {
    expect(findReferences("@Data!D:D1")).toEqual([]);
  });

  it("accepts sheet names in other alphabets", () => {
    expect(findReferences("@Übersicht!A1")[0].sheet).toBe("Übersicht");
  });

  it("ignores text that only looks like a reference", () => {
    expect(findReferences("mail me@Sheet1!A1")).toEqual([]);
    expect(findReferences("@Sheet1 and Sheet1!A1 and @Sheet1!")).toEqual([]);
  });
});

describe("formatReference", () => {
  it("leaves plain names alone", () => {
    expect(formatReference("Sheet2", "D13:G24")).toBe("@Sheet2!D13:G24");
  });

  it("quotes names with spaces or quotes", () => {
    expect(quoteSheetName("Q1 Sales")).toBe("'Q1 Sales'");
    expect(quoteSheetName("Bob's")).toBe("'Bob''s'");
  });

  it("produces text that can be found again", () => {
    const text = formatReference("Bob's data", "A1");
    expect(findReferences(text)[0].text).toBe(text);
  });
});
