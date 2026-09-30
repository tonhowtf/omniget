import { describe, expect, it } from 'vitest';
import { PoseBuffer, decodePose, emptyPose, encodePose, packQuat, seqNewer, unpackQuat, type LivePose } from './pose';
import { createGesturePlayer, validateGesture } from './gesture';
import { LiveSolver } from './solver';
import { qNorm, qRotate, type Quat, type Vec3 } from './math';
import { PoseReceiver, PoseSender, createLoopbackTransport } from './net';

const close = (a: number[], b: number[], eps: number) => a.every((v, i) => Math.abs(v - b[i]) < eps);
const sameRot = (a: Quat, b: Quat, eps = 0.01) => close(a, b, eps) || close(a, b.map((v) => -v), eps);

describe('pose codec', () => {
  it('packs quaternions in 32 bits with small error', () => {
    for (let i = 0; i < 200; i++) {
      const q = qNorm([Math.sin(i), Math.cos(i * 1.3), Math.sin(i * 0.7), Math.cos(i * 2.1)]);
      expect(sameRot(unpackQuat(packQuat(q)), q, 0.003)).toBe(true);
    }
  });

  it('encodes the upper body plus face in 64 bytes and round-trips', () => {
    const pose = emptyPose(7, 0);
    const bones = ['hips', 'spine', 'chest', 'neck', 'head', 'leftUpperArm', 'leftLowerArm', 'leftHand', 'rightUpperArm', 'rightLowerArm', 'rightHand'] as const;
    bones.forEach((b, i) => (pose.bones[b] = qNorm([0.1 * i, 0.2, -0.1, 1])));
    pose.expressions = { aa: 0.5, blinkLeft: 1, happy: 0.25 };
    pose.gaze = { yaw: 0.2, pitch: -0.1 };
    const bytes = encodePose(pose, { actor: 42 });
    expect(bytes.byteLength).toBe(64);
    const { actor, pose: back } = decodePose(bytes);
    expect(actor).toBe(42);
    expect(back.seq).toBe(7);
    expect(sameRot(back.bones.leftUpperArm!, pose.bones.leftUpperArm!, 0.003)).toBe(true);
    expect(back.expressions.aa).toBeCloseTo(0.5, 2);
    expect(back.gaze!.yaw).toBeCloseTo(0.2, 2);
    pose.blendshapes = { jawOpen: 0.5, tongueOut: 1 };
    const full = decodePose(encodePose(pose, { actor: 1, perfectSync: true })).pose;
    expect(full.blendshapes!.jawOpen).toBeCloseTo(0.5, 1);
    expect(full.blendshapes!.tongueOut).toBe(1);
  });

  it('drops stale frames by sequence, across the wrap', () => {
    expect(seqNewer(1, 0xffff)).toBe(true);
    expect(seqNewer(0xffff, 1)).toBe(false);
    const buf = new PoseBuffer(0);
    expect(buf.push(emptyPose(10, 0))).toBe(true);
    expect(buf.push(emptyPose(9, 10))).toBe(false);
    expect(buf.push(emptyPose(11, 20))).toBe(true);
  });

  it('sends through the loopback and samples an interpolated pose', () => {
    const tr = createLoopbackTransport();
    const got: LivePose[] = [];
    const rx = new PoseReceiver(tr, 0, (_a, p) => got.push(p));
    const tx = new PoseSender(tr, 3, 15);
    const p = emptyPose();
    p.bones.head = [0, 0, 0, 1];
    expect(tx.push(p, 0)).toBe(true);
    expect(tx.push(p, 10)).toBe(false);
    expect(tx.push(p, 70)).toBe(true);
    expect(got.length).toBe(2);
    expect(rx.actors()).toEqual([3]);
  });
});

describe('gestures', () => {
  it('validates clips and clamps', () => {
    const v = validateGesture('```json\n{"gesture":{"mode":"clip","clip":"wave","params":{"intensity":3}},"posture":{"mood":"tired"}}\n```');
    expect(v.ok).toBe(true);
    if (v.ok) {
      expect(v.warnings.length).toBe(1);
      const player = createGesturePlayer(v.action);
      expect(Number.isFinite(player.duration)).toBe(true);
      const mid = player.sample(player.duration / 2);
      expect(mid.bones.rightUpperArm).toBeDefined();
    }
  });

  it('rejects unknown words', () => {
    const v = validateGesture({ mode: 'compose', joints: { elbow_left: { abduct: 10 }, tail: { flex: 1 } } });
    expect(v.ok).toBe(false);
    if (!v.ok) expect(v.errors.length).toBe(2);
    expect(validateGesture({ mode: 'clip', clip: 'moonwalk' }).ok).toBe(false);
  });

  it('turns shoulder abduct 90 into an arm at shoulder height', () => {
    const v = validateGesture({ mode: 'compose', joints: { shoulder_left: { abduct: 90 } }, hold: true });
    expect(v.ok).toBe(true);
    if (!v.ok) return;
    const pose = createGesturePlayer({ ...v.action, blend: { in_s: 0, out_s: 0, layer: 'full' } }).sample(10);
    const dir = qRotate(pose.bones.leftUpperArm!, [1, 0, 0]);
    expect(close(dir, [1, 0, 0], 0.02)).toBe(true);
    const down = qRotate(pose.bones.rightUpperArm!, [-1, 0, 0]);
    expect(close(down, [0, -1, 0], 0.02)).toBe(true);
  });
});

describe('solver', () => {
  // MediaPipe world coordinates: x to the image right (the subject's left),
  // y down, z away from the camera.
  const body = (lElbow: Vec3, lWrist: Vec3) => {
    const world = new Float32Array(33 * 3);
    const vis = new Float32Array(33).fill(1);
    const set = (i: number, v: Vec3) => world.set(v, i * 3);
    set(11, [0.2, -0.5, 0]);
    set(12, [-0.2, -0.5, 0]);
    set(23, [0.1, 0, 0]);
    set(24, [-0.1, 0, 0]);
    set(13, lElbow);
    set(15, lWrist);
    set(14, [-0.2, -0.25, 0]);
    set(16, [-0.2, 0, 0]);
    for (const i of [17, 18, 19, 20, 0, 7, 8, 25, 26, 27, 28, 29, 30, 31, 32]) vis[i] = 0;
    return { world, visibility: vis };
  };

  it('keeps a T-pose arm at rest and bends the elbow forward', () => {
    const s = new LiveSolver();
    s.updateBody(body([0.45, -0.5, 0], [0.7, -0.5, 0]), 0);
    let p = s.pose(0);
    expect(qRotate(p.bones.leftUpperArm!, [1, 0, 0])[0]).toBeGreaterThan(0.95);
    const down = qRotate(p.bones.rightUpperArm!, [-1, 0, 0]);
    expect(down[1]).toBeLessThan(-0.95);

    const s2 = new LiveSolver();
    // Forearm pointing at the camera: elbow flexed 90°.
    s2.updateBody(body([0.45, -0.5, 0], [0.45, -0.5, -0.25]), 0);
    p = s2.pose(0);
    const upper = p.bones.leftUpperArm!;
    const fore = qRotate(upper, qRotate(p.bones.leftLowerArm!, [1, 0, 0]));
    expect(fore[2]).toBeGreaterThan(0.9);
  });
});
