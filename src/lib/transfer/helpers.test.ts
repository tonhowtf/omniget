import { describe, expect, it } from "vitest";
import {
  childPath,
  crumbs,
  draftOf,
  emptyForm,
  filterEntries,
  formatBytes,
  formOf,
  formReady,
  parentPath,
  progressPercent,
  secretOf,
  serverAddress,
  validName,
} from "./helpers";
import type { RemoteEntry, TransferServer } from "./types";

const nas: TransferServer = { name: "NAS", protocol: "sftp", host: "nas.local", port: 2222, user: "me", startPath: "/music", keyPath: "/k/id", hasSecret: true };

describe("remote paths", () => {
  it("walks the breadcrumb from the root to the folder", () => {
    expect(crumbs("/switch/nro")).toEqual([
      { label: "/", path: "/" },
      { label: "switch", path: "/switch" },
      { label: "nro", path: "/switch/nro" },
    ]);
    expect(crumbs("/")).toEqual([{ label: "/", path: "/" }]);
  });

  it("joins a child and finds the parent without doubling slashes", () => {
    expect(childPath("/", "switch")).toBe("/switch");
    expect(childPath("/switch", "a b.nro")).toBe("/switch/a b.nro");
    expect(parentPath("/switch/nro")).toBe("/switch");
    expect(parentPath("/switch")).toBe("/");
    expect(parentPath("/")).toBe("/");
  });

  it("refuses names that would leave the folder", () => {
    expect(validName("saves")).toBe(true);
    expect(validName(" my file.txt ")).toBe(true);
    for (const bad of ["", "  ", ".", "..", "a/b", "a\\b", "a\nb"]) expect(validName(bad)).toBe(false);
  });
});

describe("sizes and progress", () => {
  it("formats bytes with one decimal below 100 of a unit", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1023)).toBe("1023 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(300 * 1024 * 1024)).toBe("300 MB");
    expect(formatBytes(null)).toBe("");
  });

  it("gives a percentage only when the total is known", () => {
    expect(progressPercent({ bytes: 50, total: 200, status: "running" })).toBe(25);
    expect(progressPercent({ bytes: 50, total: null, status: "running" })).toBeNull();
    expect(progressPercent({ bytes: 0, total: 0, status: "running" })).toBeNull();
    expect(progressPercent({ bytes: 0, total: 0, status: "done" })).toBe(100);
    expect(progressPercent({ bytes: 300, total: 200, status: "running" })).toBe(100);
  });
});

describe("server form", () => {
  it("turns a form into a draft without the secret and without an empty port", () => {
    const form = { ...emptyForm(), name: " Switch ", host: " 192.168.0.42 ", port: "5000", secret: "x" };
    expect(draftOf(form)).toEqual({ name: "Switch", protocol: "ftp", host: "192.168.0.42", user: "", startPath: "", port: 5000 });
    expect(draftOf({ ...form, port: "" })).not.toHaveProperty("port");
    expect(JSON.stringify(draftOf(form))).not.toContain("secret");
  });

  it("keeps the key path only for sftp", () => {
    const form = { ...formOf(nas), protocol: "ftp" as const };
    expect(draftOf(form)).not.toHaveProperty("keyPath");
    expect(draftOf(formOf(nas)).keyPath).toBe("/k/id");
  });

  it("never shows a saved secret and leaves it alone when the field stays empty", () => {
    const form = formOf(nas);
    expect(form.secret).toBe("");
    expect(form.port).toBe("2222");
    expect(secretOf(form, true)).toBeNull();
    expect(secretOf({ ...form, secret: "new" }, true)).toBe("new");
    expect(secretOf(emptyForm(), false)).toBe("");
  });

  it("needs a name and a host, a user for sftp, and a real port", () => {
    expect(formReady(emptyForm())).toBe(false);
    expect(formReady({ ...emptyForm(), name: "a", host: "h" })).toBe(true);
    expect(formReady({ ...emptyForm(), name: "a", host: "h", protocol: "sftp" })).toBe(false);
    expect(formReady({ ...emptyForm(), name: "a", host: "h", port: "0" })).toBe(false);
    expect(formReady({ ...emptyForm(), name: "a", host: "h", port: "70000" })).toBe(false);
    expect(formReady({ ...emptyForm(), name: "a", host: "h", port: "21x" })).toBe(false);
  });

  it("writes the address without the default port or an empty user", () => {
    expect(serverAddress(nas)).toBe("sftp://me@nas.local:2222");
    expect(serverAddress({ protocol: "ftp", host: "10.0.0.2", port: 21, user: "" })).toBe("ftp://10.0.0.2");
  });
});

describe("filtering", () => {
  it("matches names without regard to case", () => {
    const entries: RemoteEntry[] = [
      { name: "Atmosphere", kind: "dir", size: null, modified: null },
      { name: "hbmenu.nro", kind: "file", size: 1, modified: null },
    ];
    expect(filterEntries(entries, "ATMO").map((e) => e.name)).toEqual(["Atmosphere"]);
    expect(filterEntries(entries, " ")).toHaveLength(2);
  });
});
