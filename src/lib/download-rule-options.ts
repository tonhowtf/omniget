/**
 * The Home download options once a download rule matched. Pure on purpose:
 * the Home page asks the backend for the matching rule (`preview_rule_match`)
 * and this decides what the request carries, so the precedence is testable
 * without a download.
 *
 * Precedence: what the user chose for this link wins; the rule only fills
 * what was left at its default.
 */

export type DownloadMode = "auto" | "audio" | "mute";

/** The `then` part of a rule as `preview_rule_match` returns it. */
export type RuleThen = {
  output_dir?: string | null;
  quality?: string | null;
  audio_only?: boolean | null;
  // subtitles/tags stay schema only: no Home control applies them yet.
};

export type HomeDownloadChoice = {
  outputDir: string;
  /** The user picked a folder for this link. */
  outputDirExplicit: boolean;
  mode: DownloadMode;
  /** The user picked the mode for this link (selector or preset), not the remembered one. */
  modeExplicit: boolean;
  quality: string;
  /** A specific format picked in the format list is an explicit choice too. */
  formatId: string | null;
};

export type RuleOptions = {
  outputDir: string;
  mode: DownloadMode;
  quality: string;
  /** `download_from_url` takes null for "auto". */
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
    // Kept as the page always did it: a rule quality only fills an empty selection.
    if (rule.quality && !choice.quality) {
      quality = rule.quality;
      applied.quality = true;
    }
    // Only `true` means something: an unticked box in the rule editor stores
    // `false`, which is "no preference", never "turn audio mode off".
    if (rule.audio_only === true && !choice.modeExplicit && choice.formatId === null && mode !== "audio") {
      mode = "audio";
      applied.mode = true;
    }
  }

  return { outputDir, mode, quality, downloadMode: mode === "auto" ? null : mode, applied };
}
