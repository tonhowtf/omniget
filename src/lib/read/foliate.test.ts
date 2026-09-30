import { afterEach, describe, expect, it, vi } from "vitest";
import { RemoteFile, readerCss } from "./foliate";
import { DEFAULT_PREFS } from "./prefs-defaults";

const DATA = Uint8Array.from({ length: 700_000 }, (_, i) => i % 251);

function fakeServer() {
  let calls = 0;
  const fetchMock = vi.fn(async (_url: string, init?: RequestInit) => {
    calls++;
    const range = new Headers(init?.headers).get("Range") ?? "";
    const m = /bytes=(\d+)-(\d+)/.exec(range);
    const start = m ? Number(m[1]) : 0;
    const end = m ? Number(m[2]) : DATA.length - 1;
    const body = DATA.slice(start, end + 1);
    return new Response(body, {
      status: m ? 206 : 200,
      headers: { "content-range": `bytes ${start}-${end}/${DATA.length}`, "content-type": "application/epub+zip" },
    });
  });
  vi.stubGlobal("fetch", fetchMock);
  return { calls: () => calls };
}

afterEach(() => vi.unstubAllGlobals());

describe("RemoteFile", () => {
  it("learns the size from the first range and reads across chunks", async () => {
    const srv = fakeServer();
    const f = await RemoteFile.open("http://x/b/1", "1.epub");
    expect(f.size).toBe(DATA.length);
    expect(f.type).toBe("application/epub+zip");
    const start = 262_000;
    const got = new Uint8Array(await f.slice(start, start + 5_000).arrayBuffer());
    expect(Array.from(got)).toEqual(Array.from(DATA.slice(start, start + 5_000)));
    const before = srv.calls();
    // Same chunks again come from the cache.
    await f.slice(start + 10, start + 20).arrayBuffer();
    expect(srv.calls()).toBe(before);
  });

  it("handles negative and nested slices like Blob", async () => {
    fakeServer();
    const f = await RemoteFile.open("http://x/b/1", "1.epub");
    const tail = new Uint8Array(await f.slice(-4).arrayBuffer());
    expect(Array.from(tail)).toEqual(Array.from(DATA.slice(-4)));
    const nested = new Uint8Array(await f.slice(100, 200).slice(10, 20).arrayBuffer());
    expect(Array.from(nested)).toEqual(Array.from(DATA.slice(110, 120)));
  });
});

describe("readerCss", () => {
  it("applies theme colours only when asked or on a non-light theme", () => {
    const light = readerCss({ ...DEFAULT_PREFS, theme: "light" });
    expect(light).not.toContain("#1a1a1a !important");
    const dark = readerCss({ ...DEFAULT_PREFS, theme: "dark" });
    expect(dark).toContain("color-scheme: dark");
    expect(dark).toContain("#121212");
  });
});
