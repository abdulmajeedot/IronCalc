import type { Model } from "@ironcalc/workbook";
import { describe, expect, it, vi } from "vitest";
import { FREE_MODEL_ID } from "./agent";
import {
  createWorkbookTools,
  describeReferences,
  formatRange,
  parseRange,
  toTabSeparated,
  type WorkbookRunner,
} from "./workbookTools";

const runOn =
  (model: Partial<Model>, isReadOnly = false): WorkbookRunner =>
  async (_name, mode, work) => {
    if (mode === "write" && isReadOnly) {
      throw new Error("This workbook is read-only.");
    }
    return work({ getCellLink: () => undefined, ...model } as Model);
  };

function tool(model: Partial<Model>, name: string, isReadOnly = false) {
  const found = createWorkbookTools({
    run: runOn(model, isReadOnly),
    list: () => [
      { name: "Open", isOpen: true },
      { name: "North", isOpen: false },
    ],
    create: async (requested) => requested,
  }).find((candidate) => candidate.name === name);
  if (!found) {
    throw new Error(`No tool named ${name}`);
  }
  return found;
}

describe("the model", () => {
  it("is a free one", () => {
    expect(FREE_MODEL_ID.endsWith(":free")).toBe(true);
  });
});

describe("parseRange", () => {
  it("reads single cells and ranges", () => {
    expect(parseRange("B2")).toEqual({
      row: 2,
      column: 2,
      width: 1,
      height: 1,
    });
    expect(parseRange("$a$1:C10")).toEqual({
      row: 1,
      column: 1,
      width: 3,
      height: 10,
    });
  });

  it("accepts corners in any order", () => {
    expect(formatRange(parseRange("C10:A1"))).toBe("A1:C10");
  });

  it("round-trips columns past Z", () => {
    expect(formatRange(parseRange("AA1:AZ26"))).toBe("AA1:AZ26");
    expect(formatRange(parseRange("XFD1048576"))).toBe("XFD1048576");
  });

  it("reads whole columns and rows and shows them the same way", () => {
    expect(parseRange("D:D")).toEqual({
      row: 1,
      column: 4,
      width: 1,
      height: 1_048_576,
    });
    expect(parseRange("3:5")).toEqual({
      row: 3,
      column: 1,
      width: 16_384,
      height: 3,
    });
    expect(formatRange(parseRange("D:D"))).toBe("D:D");
    expect(formatRange(parseRange("$F:$B"))).toBe("B:F");
    expect(formatRange(parseRange("3:5"))).toBe("3:5");
    expect(formatRange(parseRange("D1:D1048576"))).toBe("D:D");
    expect(formatRange(parseRange("A3:XFD3"))).toBe("3:3");
  });

  it("does not treat a partial column as a whole one", () => {
    expect(formatRange(parseRange("D2:D1048576"))).toBe("D2:D1048576");
  });

  it("rejects bad or out-of-sheet ranges", () => {
    expect(() => parseRange("A:")).toThrow();
    expect(() => parseRange("XFE:XFE")).toThrow();
    expect(() => parseRange("0:3")).toThrow();
    expect(() => parseRange("1:1048577")).toThrow();
    expect(() => parseRange("A0")).toThrow();
    expect(() => parseRange("XFE1")).toThrow();
    expect(() => parseRange("A1048577")).toThrow();
  });
});

describe("toTabSeparated", () => {
  it("quotes every field and pads short rows", () => {
    expect(toTabSeparated([["a", 'say "hi"'], ["b"]], 2)).toBe(
      '"a"\t"say ""hi"""\n"b"\t""',
    );
  });
});

describe("describeReferences", () => {
  const model = {
    getSelectedSheet: () => 0,
    getWorksheetsProperties: () => [{ name: "Sheet1" }, { name: "Data" }],
    getCellContent: (sheet: number, row: number) =>
      sheet === 1 && row === 1 ? "=1+1" : "",
    getFormattedCellValue: () => "2",
  } as unknown as Model;

  const describe_ = (target: Model, prompt: string) =>
    describeReferences(runOn(target), prompt);

  it("returns nothing without references", async () => {
    await expect(describe_(model, "no references")).resolves.toBe("");
  });

  it("includes each reference once, from its own sheet", async () => {
    await expect(
      describe_(model, "@Data!A1 and @Data!A1 and @Sheet1!B2"),
    ).resolves.toBe("@Data!A1\nA1: =1+1 (value: 2)\n\n@Sheet1!B2\n(empty)");
  });

  it("reports references it can't read", async () => {
    await expect(describe_(model, "@Nope!A1")).resolves.toContain(
      "could not be read",
    );
    const failing: WorkbookRunner = async () => {
      throw new Error("no such workbook");
    };
    await expect(describeReferences(failing, "@[Gone]")).resolves.toContain(
      "could not be read: no such workbook",
    );
  });

  it("asks for the named workbook", async () => {
    const names: (string | undefined)[] = [];
    const run: WorkbookRunner = async (name, _mode, work) => {
      names.push(name);
      return work({ getCellLink: () => undefined, ...model } as Model);
    };
    await describeReferences(run, "@[North]Data!A1 and @Sheet1!B2");
    expect(names).toEqual(["North", undefined]);
  });

  it("summarizes the sheets of a whole-file mention", async () => {
    const file = {
      ...model,
      getRowsWithData: (sheet: number, column: number) =>
        Int32Array.from(sheet === 0 && column <= 2 ? [1, 4] : []),
    } as unknown as Model;
    await expect(describe_(file, "@[North]")).resolves.toBe(
      "@[North]\nSheet Sheet1: cells in use A1:B4\nSheet Data: empty",
    );
  });

  it("shows the start of a very large range and says so", async () => {
    const text = await describe_(model, "@Data!A1:Z1000");
    expect(text).toContain("A1: =1+1");
    expect(text).toContain("Only part of this range");
  });

  it("reads only the cells of a whole column that hold data", async () => {
    const getCellContent = vi.fn(() => "x");
    const column = {
      ...model,
      getSelectedSheet: () => 0,
      getWorksheetsProperties: () => [{ name: "Data" }],
      getRowsWithData: () => Int32Array.from([3, 5]),
      getCellContent,
      getFormattedCellValue: () => "x",
    } as unknown as Model;
    await expect(describe_(column, "@Data!D:D")).resolves.toBe(
      "@Data!D:D\nD3: x\nD4: x\nD5: x",
    );
    expect(getCellContent).toHaveBeenCalledTimes(3);
  });

  it("reports an empty whole row", async () => {
    const empty = {
      ...model,
      getColumnsWithData: () => Int32Array.from([]),
    } as unknown as Model;
    await expect(describe_(empty, "@Data!3:3")).resolves.toBe(
      "@Data!3:3\n(empty)",
    );
  });
});

describe("tools", () => {
  it("refuse to change a read-only workbook", async () => {
    const pasteCsvText = vi.fn();
    const write = tool({ pasteCsvText }, "write_range", true);
    await expect(
      write.execute("id", { start: "A1", rows: [["x"]] }),
    ).rejects.toThrow("read-only");
    expect(pasteCsvText).not.toHaveBeenCalled();
  });

  it("can still read a read-only workbook", async () => {
    const read = tool(
      {
        getSelectedSheet: () => 0,
        getCellContent: () => "=1+1",
        getFormattedCellValue: () => "2",
      },
      "read_range",
      true,
    );
    const result = await read.execute("id", { range: "A1" });
    expect(result.content[0]).toEqual({
      type: "text",
      text: "A1: =1+1 (value: 2)",
    });
  });

  it("refuse ranges that are too large", async () => {
    const read = tool({ getSelectedSheet: () => 0 }, "read_range");
    await expect(read.execute("id", { range: "A1:Z1000" })).rejects.toThrow(
      "too large",
    );
  });

  it("write every row at the full width", async () => {
    const pasteCsvText = vi.fn();
    const write = tool(
      { pasteCsvText, setSelectedCell: vi.fn(), getSelectedSheet: () => 2 },
      "write_range",
    );
    await write.execute("id", { start: "B2", rows: [[1, null], ["=A1"]] });
    expect(pasteCsvText).toHaveBeenCalledWith(
      { sheet: 2, row: 2, column: 2, width: 2, height: 2 },
      '"1"\t""\n"=A1"\t""',
    );
  });

  const twoSheets = {
    getSelectedSheet: () => 0,
    getWorksheetsProperties: () =>
      [{ name: "Sheet1" }, { name: "Q1 Sales" }] as ReturnType<
        Model["getWorksheetsProperties"]
      >,
  };

  it("read a named sheet", async () => {
    const getCellContent = vi.fn(() => "7");
    const read = tool(
      { ...twoSheets, getCellContent, getFormattedCellValue: () => "7" },
      "read_range",
    );
    const result = await read.execute("id", { range: "'q1 sales'!B2" });
    expect(getCellContent).toHaveBeenCalledWith(1, 2, 2);
    expect(result.details).toEqual({ range: "'Q1 Sales'!B2" });
  });

  it("explain an unknown sheet", async () => {
    const read = tool(twoSheets, "read_range");
    await expect(read.execute("id", { range: "Nope!A1" })).rejects.toThrow(
      "The sheets are: Sheet1, Q1 Sales",
    );
  });

  it("show the sheet they write to", async () => {
    const calls: string[] = [];
    const write = tool(
      {
        ...twoSheets,
        setSelectedSheet: (sheet: number) => {
          calls.push(`sheet ${sheet}`);
        },
        setSelectedCell: () => {
          calls.push("cell");
        },
        pasteCsvText: () => {
          calls.push("paste");
        },
      },
      "write_range",
    );
    const result = await write.execute("id", {
      start: "'Q1 Sales'!C3",
      rows: [["a", "b"]],
    });
    expect(calls).toEqual(["sheet 1", "cell", "paste"]);
    expect(result.details).toEqual({ range: "'Q1 Sales'!C3:D3" });
  });

  it("label work done in another workbook", async () => {
    const read = tool(
      {
        ...twoSheets,
        getCellContent: () => "",
      },
      "read_range",
    );
    const result = await read.execute("id", {
      range: "A1",
      workbook: "North",
    });
    expect(result.details).toEqual({ range: "[North]A1" });
  });

  describe("pull_cells", () => {
    function setup() {
      const calls: string[] = [];
      const destination = {
        getSelectedSheet: () => 0,
        getWorksheetsProperties: () => [{ name: "Summary" }],
        setSelectedCell: vi.fn(),
        pasteCsvText: (_area: unknown, tsv: string) => {
          calls.push(`paste ${tsv}`);
        },
        setCellLink: (
          _sheet: number,
          row: number,
          column: number,
          link: { location: string },
        ) => {
          calls.push(`link ${row},${column} ${link.location}`);
        },
        updateRangeStyle: (_area: unknown, path: string, value: string) => {
          calls.push(`style ${path} ${value}`);
        },
      } as unknown as Model;
      const source = {
        getName: () => "North",
        getSelectedSheet: () => 0,
        getWorksheetsProperties: () => [{ name: "Sales" }],
        getCellContent: (_sheet: number, row: number) =>
          row === 2 ? "10" : row === 3 ? "=B2*2" : "",
        getFormattedCellValue: () => "20",
      } as unknown as Model;
      const pull = createWorkbookTools({
        run: async (name, _mode, work) =>
          work(name === "North" ? source : destination),
        list: () => [],
        create: async (name) => name,
      }).find((candidate) => candidate.name === "pull_cells");
      if (!pull) {
        throw new Error("No pull_cells tool");
      }
      return { pull, calls };
    }

    it("copies values and records each cell's source", async () => {
      const { pull, calls } = setup();
      const result = await pull.execute("id", {
        from_workbook: "North",
        from_range: "B2:B4",
        to_start: "C5",
      });
      expect(calls).toEqual([
        'paste "10"\n"20"\n""',
        "link 5,3 [North]Sales!B2",
        "link 6,3 [North]Sales!B3",
        "style font.u false",
        "style font.color #00803C",
      ]);
      expect(result.content[0]).toEqual({
        type: "text",
        text: "Pulled 2 cells into C5:C7.",
      });
    });

    it("refuses an empty source", async () => {
      const empty = createWorkbookTools({
        run: async (_name, _mode, work) =>
          work({
            getSelectedSheet: () => 0,
            getRowsWithData: () => Int32Array.from([]),
          } as unknown as Model),
        list: () => [],
        create: async (name) => name,
      }).find((candidate) => candidate.name === "pull_cells");
      await expect(
        empty?.execute("id", { from_range: "D:D", to_start: "A1" }),
      ).rejects.toThrow("is empty");
    });
  });

  it("show a cited cell's source when read", async () => {
    const read = tool(
      {
        getSelectedSheet: () => 0,
        getCellContent: () => "10",
        getFormattedCellValue: () => "10",
        getCellLink: () => ({
          type: "Internal",
          location: "[North]Sales!B2",
        }),
      },
      "read_range",
    );
    const result = await read.execute("id", { range: "A1" });
    expect(result.content[0]).toEqual({
      type: "text",
      text: "A1: 10 (source: [North]Sales!B2)",
    });
  });

  it("list workbooks and mark the open one", async () => {
    const result = await tool({}, "list_workbooks").execute("id", {});
    expect(result.content[0]).toEqual({
      type: "text",
      text: "Open (open)\nNorth",
    });
  });

  it("return the plan to show", async () => {
    const steps = [{ text: "Read files", done: true }];
    const result = await tool({}, "update_plan").execute("id", { steps });
    expect(result.details).toEqual({ plan: steps });
  });

  it("select the target cell before writing", async () => {
    const calls: string[] = [];
    const write = tool(
      {
        setSelectedCell: (row: number, column: number) => {
          calls.push(`select ${row},${column}`);
        },
        pasteCsvText: () => {
          calls.push("paste");
        },
        getSelectedSheet: () => 0,
      },
      "write_range",
    );
    await write.execute("id", { start: "D10", rows: [["x"]] });
    expect(calls).toEqual(["select 10,4", "paste"]);
  });
});
