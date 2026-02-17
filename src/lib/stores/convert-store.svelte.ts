export type FileStatus = "idle" | "probing" | "ready" | "converting" | "complete" | "error";

export type StreamInfo = {
  index: number;
  codec_type: string;
  codec_name: string;
  codec_long_name: string;
  width?: number;
  height?: number;
  fps?: number;
  bit_rate?: number;
  sample_rate?: number;
  channels?: number;
  duration_seconds?: number;
};

export type ProbeInfo = {
  duration_seconds: number;
  format_name: string;
  format_long_name: string;
  file_size_bytes: number;
  bit_rate: number;
  streams: StreamInfo[];
};

export type ConvertFile = {
  id: number;
  path: string;
  name: string;
  status: FileStatus;
  probe?: ProbeInfo;
  percent: number;
  conversionId?: number;
  error?: string;
  outputPath?: string;
  outputSize?: number;
};

export type ConvertOptions = {
  outputFormat: string;
  videoCodec: string;
  audioCodec: string;
  resolution: string;
  preset: string;
  videoBitrate: string;
  audioBitrate: string;
  trimStart: string;
  trimEnd: string;
};

export type HwAccelInfo = {
  encoders: string[];
  decoders: string[];
  recommended_video_encoder: string | null;
  recommended_decoder: string | null;
};

const defaultOptions: ConvertOptions = {
  outputFormat: "mp4",
  videoCodec: "auto",
  audioCodec: "auto",
  resolution: "original",
  preset: "medium",
  videoBitrate: "",
  audioBitrate: "",
  trimStart: "",
  trimEnd: "",
};

let nextId = 1;
let files: ConvertFile[] = $state([]);
let options: ConvertOptions = $state({ ...defaultOptions });
let hwAccel: HwAccelInfo | null = $state(null);

export function getFiles(): ConvertFile[] {
  return files;
}

export function getOptions(): ConvertOptions {
  return options;
}

export function getHwAccel(): HwAccelInfo | null {
  return hwAccel;
}

export function addFiles(paths: string[]) {
  for (const path of paths) {
    if (files.some((f) => f.path === path)) continue;
    const name = path.split(/[/\\]/).pop() ?? path;
    files = [
      ...files,
      {
        id: nextId++,
        path,
        name,
        status: "idle",
        percent: 0,
      },
    ];
  }
}

export function removeFile(id: number) {
  files = files.filter((f) => f.id !== id);
}

export function clearFiles() {
  files = [];
}

export function updateFileProbe(id: number, probe: ProbeInfo) {
  files = files.map((f) =>
    f.id === id ? { ...f, probe, status: "ready" as FileStatus } : f,
  );
}

export function markFileProbing(id: number) {
  files = files.map((f) =>
    f.id === id ? { ...f, status: "probing" as FileStatus } : f,
  );
}

export function markFileConverting(id: number, conversionId: number) {
  files = files.map((f) =>
    f.id === id
      ? { ...f, status: "converting" as FileStatus, conversionId, percent: 0 }
      : f,
  );
}

export function updateFileProgress(conversionId: number, percent: number) {
  files = files.map((f) =>
    f.conversionId === conversionId ? { ...f, percent } : f,
  );
}

export function markFileComplete(
  conversionId: number,
  outputPath: string,
  outputSize: number,
) {
  files = files.map((f) =>
    f.conversionId === conversionId
      ? {
          ...f,
          status: "complete" as FileStatus,
          percent: 100,
          outputPath,
          outputSize,
        }
      : f,
  );
}

export function markFileError(conversionId: number, error: string) {
  files = files.map((f) =>
    f.conversionId === conversionId
      ? { ...f, status: "error" as FileStatus, error }
      : f,
  );
}

export function updateOptions(partial: Partial<ConvertOptions>) {
  options = { ...options, ...partial };
}

export function setHwAccel(info: HwAccelInfo) {
  hwAccel = info;
}

export function resetOptions() {
  options = { ...defaultOptions };
}
