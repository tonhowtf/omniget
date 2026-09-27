import { describe, expect, it } from "vitest";
import { durationLabel, planItemId, sortTasks, taskKey, type MaintenanceTask } from "./maintenance";

function task(p: Partial<MaintenanceTask>): MaintenanceTask {
  return { id: "x", title: "", what: "", admin: false, minutes: 1, long: false, group: "network", available: true, command: "", undo: "none", ...p };
}

describe("maintenance", () => {
  it("sorts available first, then group, then quick before long", () => {
    const s = sortTasks([
      task({ id: "verify", group: "disk", long: true, minutes: 15 }),
      task({ id: "gone", group: "network", available: false }),
      task({ id: "dns", group: "network" }),
      task({ id: "ql", group: "apps" }),
    ]);
    expect(s.map((t) => t.id)).toEqual(["dns", "ql", "verify", "gone"]);
  });

  it("labels durations", () => {
    expect(durationLabel(1).key).toMatch(/quick$/);
    expect(durationLabel(20)).toEqual({ key: "washmybelly.maintenance.duration.minutes", count: 20 });
    expect(durationLabel(60)).toEqual({ key: "washmybelly.maintenance.duration.hours", count: 1 });
  });

  it("names plan items and text keys", () => {
    expect(planItemId("dns_flush")).toBe("maintenance:dns_flush");
    expect(taskKey("sfc_scan", "title")).toBe("washmybelly.maintenance.task.sfc_scan.title");
  });
});
