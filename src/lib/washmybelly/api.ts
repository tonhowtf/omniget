import { invoke } from "@tauri-apps/api/core";
import type { Caps } from "$components/washmybelly/registry";

export type Risk = "read" | "reversible" | "destructive" | "extreme";

type PlanItemPreview = {
  id: string;
  label: string;
  detail?: string | null;
  group: string;
  bytes: number;
  risk: Risk;
  admin: boolean;
  action: { type: string; path?: string; destination?: string; argv?: string[]; module?: string; action?: string };
};

export type PlanPreview = {
  plan_id: string;
  kind: string;
  status: "ready" | "pending_approval" | "executing" | "executed" | "denied" | "expired";
  created: number;
  expires: number;
  item_count: number;
  total_bytes: number;
  needs_admin: boolean;
  max_risk: Risk;
  by_risk: Record<string, { count: number; bytes: number }>;
  by_group: Record<string, { count: number; bytes: number }>;
  receipt_id?: string | null;
  items: PlanItemPreview[];
  items_truncated: boolean;
};

export type ReceiptSummary = {
  receipt_id: string;
  plan_id: string;
  kind: string;
  created: number;
  bytes_freed: number;
  done: number;
  skipped: number;
  failed: number;
  restorable: boolean;
  restored_at?: number | null;
  items?: { item_id: string; label: string; status: "done" | "skipped" | "failed"; detail: string; bytes: number; undo?: string | null }[];
};

export type Progress = { task: string; stage: string; done: number; total: number; message: string };

export function capabilities(refresh = false): Promise<Caps> {
  return invoke<Caps>("washmybelly_capabilities", { refresh });
}

export function call<T = unknown>(tool: string, input: Record<string, unknown> = {}): Promise<T> {
  return invoke<T>("washmybelly_call", { tool, input });
}

export function ui<T = unknown>(action: string, input: Record<string, unknown> = {}): Promise<T> {
  return invoke<T>("washmybelly_ui", { action, input });
}

export function plan(kind: string, itemIds: string[]): Promise<PlanPreview> {
  return call<PlanPreview>("wmb_plan", { kind, item_ids: itemIds });
}

export function execute(planId: string): Promise<ReceiptSummary> {
  return call<ReceiptSummary>("wmb_execute", { plan_id: planId });
}

export function restore(receiptId: string): Promise<{ receipt_id: string; items: { label: string; ok: boolean; detail: string }[] }> {
  return call("wmb_restore", { receipt_id: receiptId });
}

export function monitorStart(): Promise<void> {
  return invoke("washmybelly_monitor_start");
}

export function monitorStop(): Promise<void> {
  return invoke("washmybelly_monitor_stop");
}

export { formatBytes } from "./format";
