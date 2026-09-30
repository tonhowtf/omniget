// invoke wrappers of the avatar commands (src-tauri/src/commands/city3d/avatar.rs).
import { invoke } from '@tauri-apps/api/core';
import type { Catalog } from './catalog';

export interface Identity {
  name: string; bio: string; big_five: [number, number, number, number, number];
  profession: string; home: string;
}
export interface Resident { npc_id: number; identity: Identity; genome: string }
export interface AvatarRecord {
  genome: string;
  identity?: Identity | null;
  extended?: unknown;
  catalog_hash?: string;
  updated_ms?: number;
}
export interface Traits {
  attributes?: { trait_type: string; value: string | number }[];
  phenotypes?: string[];
}

export const avatarCatalog = () =>
  invoke<{ hash: string; catalog: Catalog; schema: unknown }>('city3d_avatar_catalog');
export const genomeRandom = (seed?: number) =>
  invoke<{ genome: string; catalog_hash: string }>('city3d_genome_random', { seed });
export const genomeFromTraits = (traits: Traits, seed?: number) =>
  invoke<{ genome: string; catalog_hash: string }>('city3d_genome_from_traits', { traits, seed });
export const avatarGet = () => invoke<AvatarRecord | null>('city3d_avatar_get');
export const avatarSet = (record: AvatarRecord) => invoke<AvatarRecord>('city3d_avatar_set', { record });
export const npcIdentities = (cityId: string, count: number, seed?: number) =>
  invoke<Resident[]>('city3d_npc_identities', { cityId, count, seed });
export const exportVrmFile = (path: string, dataBase64: string) =>
  invoke<number>('city3d_avatar_export_vrm', { path, dataBase64 });
