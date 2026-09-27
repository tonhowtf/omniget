import { describe, expect, it } from "vitest";
import { appName, mergeTargets, orderServices, resetCommand, serviceKey, type TccApp, type TccService } from "./tcc";

const app = (p: Partial<TccApp>): TccApp => ({ client: "com.example.App", client_type: 0, auth: "allowed", db: "user", can_reset: true, ...p });

describe("tcc", () => {
  it("names apps from bundle ids and paths", () => {
    expect(appName("us.zoom.xos")).toBe("Zoom");
    expect(appName("com.googlecode.iterm2")).toBe("iTerm2");
    expect(appName("/usr/local/bin/ffmpeg")).toBe("ffmpeg");
    expect(appName("/Applications/Foo Bar.app")).toBe("Foo Bar");
    expect(appName("com.microsoft.teams2")).toBe("Teams2");
  });

  it("orders services with grants first and can hide denied", () => {
    const list: TccService[] = [
      { service: "Camera", key: "camera", allowed: 0, apps: [app({ auth: "denied" })] },
      { service: "Microphone", key: "microphone", allowed: 1, apps: [app({})] },
    ];
    expect(orderServices(list).map((s) => s.service)).toEqual(["Microphone", "Camera"]);
    expect(orderServices(list, true).map((s) => s.service)).toEqual(["Microphone"]);
    expect(serviceKey(list[0])).toBe("washmybelly.privacy.tcc.service.camera");
  });

  it("merges automation targets into one row", () => {
    const rows = mergeTargets([app({ client: "com.x.Robot", target: "com.apple.finder", auth: "limited" }), app({ client: "com.x.Robot", target: "com.apple.mail" }), app({ client: "com.a.Alpha" })]);
    expect(rows.map((r) => r.client)).toEqual(["com.a.Alpha", "com.x.Robot"]);
    expect(rows[1].targets).toEqual(["com.apple.finder", "com.apple.mail"]);
    expect(rows[1].auth).toBe("allowed");
    expect(resetCommand("Camera", "us.zoom.xos")).toBe("tccutil reset Camera us.zoom.xos");
  });
});
