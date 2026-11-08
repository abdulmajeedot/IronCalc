import "fake-indexeddb/auto";
import { beforeEach, describe, expect, it } from "vitest";
import { getVersionBytes, listVersions, saveVersion } from "./versions";

function deleteDatabase(): Promise<void> {
  return new Promise((resolve) => {
    const request = indexedDB.deleteDatabase("ironcalc-ai");
    request.onsuccess = () => resolve();
    request.onerror = () => resolve();
    request.onblocked = () => resolve();
  });
}

const save = (uuid: string, request: string, byte = 1) =>
  saveVersion({ uuid, name: "Book", request, bytes: Uint8Array.of(byte) });

describe("versions", () => {
  beforeEach(deleteDatabase);

  it("lists a workbook's versions, newest first, without contents", async () => {
    await save("a", "first");
    await save("a", "second");
    await save("b", "other");
    const versions = await listVersions("a");
    expect(versions.map((item) => item.request)).toEqual(["second", "first"]);
    expect(versions[0]).not.toHaveProperty("bytes");
  });

  it("returns the saved contents", async () => {
    await save("a", "first", 7);
    const [version] = await listVersions("a");
    expect(Array.from((await getVersionBytes(version.id)) ?? [])).toEqual([7]);
  });

  it("keeps only the newest versions of a workbook", async () => {
    for (let index = 0; index < 23; index += 1) {
      await save("a", `request ${index}`);
    }
    const versions = await listVersions("a");
    expect(versions).toHaveLength(20);
    expect(versions[0].request).toBe("request 22");
    expect(versions[19].request).toBe("request 3");
  });
});
