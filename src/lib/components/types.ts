export type RegistrySource = {
  id: string;
  name: string;
  homepage: string;
  index_url: string;
  item_url: string;
  docs_url: string | null;
  custom?: boolean;
};

export type ItemSummary = {
  key: string;
  registryId: string;
  name: string;
  title: string;
  description: string;
  type: string;
  categories: string[];
  dependencies: string[];
  registryDependencies: string[];
  fileCount: number;
};

export type ItemFile = {
  path: string;
  type: string;
  target: string | null;
  content: string;
};

export type ItemDetail = {
  name: string;
  title: string;
  description: string;
  type: string;
  dependencies: string[];
  devDependencies: string[];
  registryDependencies: string[];
  files: ItemFile[];
  cssVarCount: number;
};

export type ParsedRegistry = {
  name: string | null;
  homepage: string | null;
  items: ItemSummary[];
  skipped: number;
};

export type FetchError = { code: string; detail: string | null };

export type FetchResult = {
  url: string;
  body: unknown;
  fetched_at: number;
  from_cache: boolean;
  stale: boolean;
  offline_error: FetchError | null;
};

export type ProjectInfo = {
  path: string;
  name: string;
  has_package_json: boolean;
  lockfiles: string[];
  has_components_json: boolean;
  components_json_error: boolean;
  css_path: string | null;
  css_exists: boolean;
  tailwind_config: string | null;
  yarn_classic?: boolean;
};

export type PackageManager = "npm" | "pnpm" | "yarn" | "bun";

export type BuiltCommand = { program: string; args: string[]; display: string };

export type RunLine = { stream: "stdout" | "stderr"; line: string };

export type RunOutcome = { ok: boolean; code: number | null; cancelled: boolean; error: string | null; output?: RunLine[] };

export type OutputLine = { run_id: string; stream: "stdout" | "stderr"; line: string };
