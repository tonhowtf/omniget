export type ToolName =
  | "research_endpoints"
  | "research_call"
  | "research_usage"
  | "research_tiktok_creators"
  | "research_tiktok_brief"
  | "research_meta_ads"
  | "research_reels";

export type Result = Record<string, unknown>;

export interface EndpointParam {
  name: string;
  in: string;
  required: boolean;
  type: string;
  enum?: string[];
  description?: string;
  example?: string;
}

export interface Operation {
  id: string;
  platform: string;
  group: string;
  method: string;
  path: string;
  summary: string;
  params: EndpointParam[];
  pagination: string[];
  credits: string | null;
}

export interface PlatformRow {
  platform: string;
  name: string;
  operations: number;
}

export type WorkflowId = "creators" | "brief" | "ads" | "reels";

export type FieldKind = "text" | "area" | "lines" | "number" | "select" | "toggle";

export interface Field {
  key: string;
  kind: FieldKind;
  options?: string[];
  min?: number;
  max?: number;
  initial?: string;
  ops?: string[];
  placeholder?: string;
}

export interface Workflow {
  id: WorkflowId;
  tool: ToolName;
  fields: Field[];
}

export interface LinkCell {
  href: string;
  label: string;
}

export type Cell = string | number | null | LinkCell | string[];

export type ColumnKind = "text" | "number" | "link" | "flags";

export interface Column {
  key: string;
  kind: ColumnKind;
}

export interface Table {
  id: string;
  columns: Column[];
  rows: Record<string, Cell>[];
}
