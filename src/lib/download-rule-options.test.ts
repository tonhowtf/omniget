import { describe, expect, it } from "vitest";
import { applyRuleToDownloadOptions, type HomeDownloadChoice } from "./download-rule-options";

const base: HomeDownloadChoice = {
  outputDir: "/tmp/fixture/downloads",
  outputDirExplicit: false,
  mode: "auto",
  modeExplicit: false,
  quality: "best",
  formatId: null,
};

describe("applyRuleToDownloadOptions", () => {
  it("keeps the choice when no rule matched", () => {
    const r = applyRuleToDownloadOptions(base, null);
    expect(r).toMatchObject({ outputDir: base.outputDir, mode: "auto", quality: "best", downloadMode: null });
    expect(r.applied).toEqual({ outputDir: false, mode: false, quality: false });
  });

  it("turns audio_only into audio mode when the user did not pick a mode", () => {
    const r = applyRuleToDownloadOptions(base, { audio_only: true });
    expect(r.mode).toBe("audio");
    expect(r.downloadMode).toBe("audio");
    expect(r.applied.mode).toBe(true);
  });

  it("overrides the remembered mode from the last session", () => {
    const r = applyRuleToDownloadOptions({ ...base, mode: "mute" }, { audio_only: true });
    expect(r.downloadMode).toBe("audio");
  });

  it("lets an explicit mode win over the rule", () => {
    for (const mode of ["auto", "mute"] as const) {
      const r = applyRuleToDownloadOptions({ ...base, mode, modeExplicit: true }, { audio_only: true });
      expect(r.mode).toBe(mode);
      expect(r.applied.mode).toBe(false);
    }
  });

  it("lets an explicit format win over the rule", () => {
    const r = applyRuleToDownloadOptions({ ...base, formatId: "137+140" }, { audio_only: true });
    expect(r.downloadMode).toBeNull();
  });

  it("treats audio_only false or missing as no preference", () => {
    expect(applyRuleToDownloadOptions({ ...base, mode: "audio" }, { audio_only: false }).mode).toBe("audio");
    expect(applyRuleToDownloadOptions(base, { audio_only: null }).mode).toBe("auto");
    expect(applyRuleToDownloadOptions(base, {}).applied.mode).toBe(false);
  });

  it("keeps the output folder and quality precedence the page already had", () => {
    const rule = { output_dir: "/tmp/fixture/rule", quality: "720p" };
    expect(applyRuleToDownloadOptions(base, rule).outputDir).toBe("/tmp/fixture/rule");
    expect(applyRuleToDownloadOptions({ ...base, outputDirExplicit: true }, rule).outputDir).toBe(base.outputDir);
    expect(applyRuleToDownloadOptions(base, rule).quality).toBe("best");
    expect(applyRuleToDownloadOptions({ ...base, quality: "" }, rule).quality).toBe("720p");
  });

  it("combines output folder and audio mode from one rule", () => {
    const r = applyRuleToDownloadOptions(base, { output_dir: "/tmp/fixture/music", audio_only: true });
    expect(r).toMatchObject({ outputDir: "/tmp/fixture/music", downloadMode: "audio" });
  });
});
