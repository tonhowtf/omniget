import { describe, expect, it } from "vitest";
import {
  addTick,
  emptySeries,
  errorCode,
  filterProcesses,
  formatRate,
  formatTemp,
  linePath,
  machineCpu,
  niceMax,
  pushSample,
  scoreTone,
  sortProcesses,
  toggleSort,
  type LiveTick,
  type ProcessRow,
} from "./monitor";

const row = (pid: number, name: string, cpu: number, mem: number, disk = 0, user = "tonho"): ProcessRow => ({
  pid,
  name,
  user,
  cpu_percent: cpu,
  memory_bytes: mem,
  disk_read_per_s: disk,
  disk_write_per_s: 0,
  command: `/usr/bin/${name} --flag`,
  started: 1,
  status: "Running",
  mine: true,
  can_end: true,
});

describe("series", () => {
  it("keeps the last samples and skips missing readings", () => {
    let b: number[] = [];
    for (let i = 0; i < 5; i++) b = pushSample(b, i, 3);
    expect(b).toEqual([2, 3, 4]);
    expect(pushSample(b, null, 3)).toBe(b);
    expect(pushSample(b, NaN, 3)).toBe(b);
  });

  it("adds a live tick to every chart", () => {
    const t: LiveTick = {
      t: 1,
      cpu: { total: 40, cores: [30, 50] },
      memory: { total: 200, used: 50, available: 150, swap_total: 0, swap_used: 0 },
      net: { rx_per_s: 10, tx_per_s: 20 },
      disk: { read_per_s: 5, write_per_s: 6 },
      gpu_percent: null,
      temp_c: 55,
    };
    const s = addTick(emptySeries(), t);
    expect(s.cpu).toEqual([40]);
    expect(s.memory).toEqual([25]);
    expect(s.gpu).toEqual([]);
    expect(s.temp).toEqual([55]);
    expect(s.tx).toEqual([20]);
  });

  it("rounds the chart ceiling to a steady step", () => {
    expect(niceMax([3, 7])).toBe(10);
    expect(niceMax([120])).toBe(200);
    expect(niceMax([0], 100)).toBe(100);
    expect(niceMax([1_300_000])).toBe(2_000_000);
  });

  it("draws the line from the right edge", () => {
    const { line, area } = linePath([0, 100], 100, 100, 50, 3);
    expect(line).toBe("M50.0,50.0 L100.0,0.0");
    expect(area.endsWith("Z")).toBe(true);
    expect(linePath([], 100, 100, 50).line).toBe("");
  });
});

describe("formatting", () => {
  it("formats rates and temperatures", () => {
    expect(formatRate(0)).toBe("0 B/s");
    expect(formatRate(1_500_000)).toBe("1.5 MB/s");
    expect(formatTemp(null)).toBe("—");
    expect(formatTemp(64.6)).toBe("65 °C");
    expect(machineCpu(200, 10)).toBe(20);
    expect(scoreTone(97)).toBe("good");
    expect(scoreTone(50)).toBe("poor");
  });

  it("reads the refusal code of the core", () => {
    expect(errorCode("REFUSED: the process belongs to another user")).toBe("REFUSED");
    expect(errorCode(new Error("CHANGED: that PID now belongs to another process"))).toBe("CHANGED");
    expect(errorCode("boom")).toBe("OTHER");
  });
});

describe("processes", () => {
  const rows = [row(3, "Safari", 10, 500), row(1, "node", 50, 100, 9), row(2, "Mail", 10, 900)];

  it("sorts by cpu with ties by pid, and by name", () => {
    expect(sortProcesses(rows, "cpu", "desc").map((r) => r.pid)).toEqual([1, 2, 3]);
    expect(sortProcesses(rows, "memory", "desc").map((r) => r.pid)).toEqual([2, 3, 1]);
    expect(sortProcesses(rows, "name", "asc").map((r) => r.name)).toEqual(["Mail", "node", "Safari"]);
    expect(sortProcesses(rows, "disk", "desc")[0].pid).toBe(1);
  });

  it("filters by name, command, pid or user", () => {
    expect(filterProcesses(rows, "saf").map((r) => r.pid)).toEqual([3]);
    expect(filterProcesses(rows, "2").map((r) => r.pid)).toEqual([2]);
    expect(filterProcesses(rows, "/usr/bin/node").map((r) => r.pid)).toEqual([1]);
    expect(filterProcesses(rows, " ")).toBe(rows);
  });

  it("toggles sort direction", () => {
    expect(toggleSort({ key: "cpu", dir: "desc" }, "cpu")).toEqual({ key: "cpu", dir: "asc" });
    expect(toggleSort({ key: "cpu", dir: "desc" }, "name")).toEqual({ key: "name", dir: "asc" });
    expect(toggleSort({ key: "name", dir: "asc" }, "memory")).toEqual({ key: "memory", dir: "desc" });
  });
});

describe("factors", () => {
  it("formats the reading of each factor", async () => {
    const { factorValue } = await import("./monitor");
    const f = (id: string, value: number) => ({ id, value, penalty: 0, severity: "ok", detail: "" }) as never;
    expect(factorValue(f("disk_space", 24.1), String)).toBe("24%");
    expect(factorValue(f("temperature", 64.7), String)).toBe("65 °C");
    expect(factorValue(f("junk", 5), (n) => `${n} B`)).toBe("5 B");
    expect(factorValue(f("login_items", 7), String)).toBe("7");
  });
});
