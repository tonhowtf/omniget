export type Probe = {
  path: string;
  name: string;
  size_bytes: number;
  duration_ms: number;
  width: number;
  height: number;
  fps: number;
  has_video: boolean;
  has_audio: boolean;
  video_codec: string | null;
};

export type Tools = { ffmpeg: boolean; subtitles: boolean; drawtext: boolean };

export type Word = { text: string; start_ms: number; end_ms: number };

export type Segment = {
  id: string;
  start_ms: number;
  end_ms: number;
  text: string;
  words: Word[];
  estimated: boolean;
};

export type RawSegment = { start_ms: number; end_ms: number; text: string; words?: Word[] };

export type Transcript = { lang: string; model: string; segments: RawSegment[] };

export type AsrModel = { id: string; label: string; bytes: number; downloaded: boolean; default: boolean };

export type AsrStatus = {
  runtime_installed: boolean;
  runtime_downloadable: boolean;
  default_model: string;
  ready: boolean;
  models: AsrModel[];
};

export type AsrProgress = { model: string; id: string; stage: string; done: number; total: number | null };

export type Aspect = "keep" | "9:16" | "1:1" | "4:5";
export type FrameMode = "crop" | "fit";
export type Quality = "fast" | "balanced" | "high";
export type Position = "bottom" | "middle" | "top";
export type Align = "center" | "left";
export type PresetId = "clean" | "bold_pop" | "boxed" | "minimal";

export type CaptionStyle = {
  preset: PresetId;
  font: string;
  size: number;
  bold: boolean;
  color: string;
  highlight: string;
  karaoke: boolean;
  outline: number;
  outlineColor: string;
  box: boolean;
  boxColor: string;
  shadow: number;
  position: Position;
  align: Align;
  maxChars: number;
  maxLines: 1 | 2;
  uppercase: boolean;
};

export type Reframe = { aspect: Aspect; mode: FrameMode; position: number };

export type IntroSettings = {
  enabled: boolean;
  seconds: number;
  title: string;
  subtitle: string;
  background: string;
  color: string;
  fade: boolean;
};

export type IntroRequest = {
  seconds: number;
  title_lines: string[];
  subtitle_lines: string[];
  background: string;
  color: string;
  fade: boolean;
};

export type RenderRequest = {
  input: string;
  out_dir: string | null;
  ass: string | null;
  reframe: Reframe;
  intro: IntroRequest | null;
  fade_out: boolean;
  quality: Quality;
};

export type RenderOutcome = { output: string; size_bytes: number; duration_ms: number; width: number; height: number };

export type RenderStage = "preparing" | "rendering" | "done";

export type RenderProgress = { job_id: string; percent: number; stage: RenderStage };

export type TranscribeProgress = { job_id: string; percent: number };
