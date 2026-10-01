type DownloadMode = "auto" | "audio" | "mute";

export type RuleThen = {
  output_dir?: string | null;
  quality?: string | null;
  audio_only?: boolean | null;
};

type HomeDownloadChoice = {
  outputDir: string;
  outputDirExplicit: boolean;
  mode: DownloadMode;
  modeExplicit: boolean;
  quality: string;
  formatId: string | null;
};

type RuleOptions = {
  outputDir: string;
  mode: DownloadMode;
  quality: string;
  downloadMode: "audio" | "mute" | null;
  applied: { outputDir: boolean; mode: boolean; quality: boolean };
};

export function applyRuleToDownloadOptions(choice: HomeDownloadChoice, rule: RuleThen | null | undefined): RuleOptions {
  let { outputDir, mode, quality } = choice;
  const applied = { outputDir: false, mode: false, quality: false };

  if (rule) {
    if (rule.output_dir && !choice.outputDirExplicit) {
      outputDir = rule.output_dir;
      applied.outputDir = true;
    }
    if (rule.quality && !choice.quality) {
      quality = rule.quality;
      applied.quality = true;
    }
    if (rule.audio_only === true && !choice.modeExplicit && choice.formatId === null && mode !== "audio") {
      mode = "audio";
      applied.mode = true;
    }
  }

  return { outputDir, mode, quality, downloadMode: mode === "auto" ? null : mode, applied };
}
