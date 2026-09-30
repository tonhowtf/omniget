// Residents and avatars of the 3D city (agent D). The city view (H) uses
// `createCharacterSystem`; the creator and the exporter use the rest.
export { createCharacterSystem } from './system';
export type { CharacterSystem, CharacterSystemOptions, TargetExtra, TierName, Vec3 } from './system';
export { loadAnny, buildBody, parseAnny, phenotypeCoefficients } from './anny';
export type { AnnyBase, BodyShape } from './anny';
export { buildCharacter, pieceLicenses } from './assemble';
export type { CharacterRig, Tier } from './assemble';
export { catalog } from './catalog';
export type { Catalog, Piece, HairStyle } from './catalog';
export { decodeGenome, encodeGenome, validateGenome, fixGenome, pheno, NO_FACE } from './genome';
export type { Genome } from './genome';
export { loadClips, generateClips, CITY_ACTIONS, GESTURES } from './clips';
export type { Clip, ClipJson } from './clips';
export { Animator, composeClip } from './animator';
export { poseToBones, resolvePose, JOINT_LIMITS, ACTIONS } from './posecode';
export type { GestureAction, JointPose, Timing } from './posecode';
export { encodePoseFrame, decodePoseFrame, STREAM_BONES, STREAM_EXPRESSIONS } from './livepose';
export type { LivePose } from './livepose';
export { VRM_PRESETS, MOOD_FACE, toArkit } from './expressions';
export { exportVrm, mergePermissions, licensePermissions } from './vrm-export';
export type { VrmMetaInput, VrmPermissions, ExportResult } from './vrm-export';
export { VRM_BONES } from './rig';
export type { VrmBone } from './rig';
export * as avatarApi from './api';
