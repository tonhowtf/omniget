// Live expression: webcam → avatar, LLM gestures, pose on the network.
// The character system consumes `LivePose` through `applyPose(id, pose)`.

export * from './pose';
export { startLive, listCameras, LiveError, isLinux } from './live';
export type { LiveOptions, LiveSession, LiveStats, LiveMode, PoseStream, CameraInfo, LiveErrorCode } from './live';
export { LiveSolver, blendshapesToExpressions, JOINT_LIMITS, applyLimit } from './solver';
export type { FaceFrame, BodyFrame, HandsFrame, SolverOptions } from './solver';
export {
  validateGesture,
  createGesturePlayer,
  gestureFromText,
  jointsToPose,
  clipPhases,
  clipCatalogPrompt,
  CLIP_CATALOG,
  JOINT_NAMES,
  ACTIONS as GESTURE_ACTIONS,
  ROM as GESTURE_ROM,
} from './gesture';
export type { GestureAction, GesturePlayer, GestureSpec, GestureValidation, ClipSpec } from './gesture';
export { createLoopbackTransport, createWebSocketTransport, PoseSender, PoseReceiver } from './net';
export type { PoseTransport } from './net';
export { assetStatus, loadAsset, LIVE_ASSETS } from './assets';
export type { AssetProgress, AssetStatus } from './assets';
