import { invoke } from '@tauri-apps/api/core';

export type ArtifactKind = 'html' | 'md' | 'svg' | 'link';

export interface Artifact {
  id: string;
  name: string;
  kind: ArtifactKind;
  createdAt: string;
  source: string;
  origin: string;
  file?: string;
  url?: string;
  size: number;
  session?: string;
  path?: string;
  localUrl?: string;
}

export const artifactsPanel = $state<{ open: boolean; at: { x: number; y: number } | null }>({
  open: false,
  at: null,
});

export function openArtifacts(at: { x: number; y: number } | null = null): void {
  artifactsPanel.at = at;
  artifactsPanel.open = true;
}

export function closeArtifacts(): void {
  artifactsPanel.open = false;
}

export function artifactList(workspaceId: string): Promise<Artifact[]> {
  return invoke<Artifact[]>('canvas_artifact_list', { workspaceId });
}

export function artifactOpen(
  workspaceId: string,
  floorId: string,
  artifactId: string,
  at: { x: number; y: number } | null,
): Promise<{ nodeId: string }> {
  return invoke<{ nodeId: string }>('canvas_artifact_open', { workspaceId, floorId, artifactId, at });
}
