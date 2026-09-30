// Small vector and quaternion helpers for the live pose path. Plain tuples so
// the vision worker and the wire codec do not pull three.js in.

export type Vec3 = [number, number, number];
/** Quaternion as [x, y, z, w]. */
export type Quat = [number, number, number, number];

export const QI: Quat = [0, 0, 0, 1];
export const DEG = Math.PI / 180;

export const sub = (a: Vec3, b: Vec3): Vec3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
export const add = (a: Vec3, b: Vec3): Vec3 => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
export const scale = (a: Vec3, s: number): Vec3 => [a[0] * s, a[1] * s, a[2] * s];
export const dot = (a: Vec3, b: Vec3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
export const cross = (a: Vec3, b: Vec3): Vec3 => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
];
export const len = (a: Vec3): number => Math.hypot(a[0], a[1], a[2]);
export const mid = (a: Vec3, b: Vec3): Vec3 => [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2, (a[2] + b[2]) / 2];

export function norm(a: Vec3): Vec3 {
  const l = len(a);
  return l > 1e-9 ? [a[0] / l, a[1] / l, a[2] / l] : [0, 0, 0];
}

export const clamp = (v: number, lo: number, hi: number): number => (v < lo ? lo : v > hi ? hi : v);
export const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;

export function qMul(a: Quat, b: Quat): Quat {
  const [ax, ay, az, aw] = a;
  const [bx, by, bz, bw] = b;
  return [
    aw * bx + ax * bw + ay * bz - az * by,
    aw * by - ax * bz + ay * bw + az * bx,
    aw * bz + ax * by - ay * bx + az * bw,
    aw * bw - ax * bx - ay * by - az * bz,
  ];
}

export const qConj = (q: Quat): Quat => [-q[0], -q[1], -q[2], q[3]];
export const qDot = (a: Quat, b: Quat): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];

export function qNorm(q: Quat): Quat {
  const l = Math.hypot(q[0], q[1], q[2], q[3]);
  return l > 1e-9 ? [q[0] / l, q[1] / l, q[2] / l, q[3] / l] : [0, 0, 0, 1];
}

export function qAxisAngle(axis: Vec3, angle: number): Quat {
  const n = norm(axis);
  const s = Math.sin(angle / 2);
  return [n[0] * s, n[1] * s, n[2] * s, Math.cos(angle / 2)];
}

/** Rotates `v` by the unit quaternion `q`. */
export function qRotate(q: Quat, v: Vec3): Vec3 {
  const u: Vec3 = [q[0], q[1], q[2]];
  const w = q[3];
  const t = scale(cross(u, v), 2);
  return add(add(v, scale(t, w)), cross(u, t));
}

/** Shortest rotation taking unit vector `a` onto unit vector `b`. */
export function qFromUnitVectors(a: Vec3, b: Vec3): Quat {
  let r = dot(a, b) + 1;
  let x: number, y: number, z: number;
  if (r < 1e-6) {
    r = 0;
    if (Math.abs(a[0]) > Math.abs(a[2])) {
      x = -a[1];
      y = a[0];
      z = 0;
    } else {
      x = 0;
      y = -a[2];
      z = a[1];
    }
  } else {
    [x, y, z] = cross(a, b);
  }
  return qNorm([x, y, z, r]);
}

/** Quaternion of the rotation matrix whose columns are the basis `x`, `y`, `z`. */
export function qFromBasis(x: Vec3, y: Vec3, z: Vec3): Quat {
  const m00 = x[0], m10 = x[1], m20 = x[2];
  const m01 = y[0], m11 = y[1], m21 = y[2];
  const m02 = z[0], m12 = z[1], m22 = z[2];
  const trace = m00 + m11 + m22;
  let q: Quat;
  if (trace > 0) {
    const s = 0.5 / Math.sqrt(trace + 1);
    q = [(m21 - m12) * s, (m02 - m20) * s, (m10 - m01) * s, 0.25 / s];
  } else if (m00 > m11 && m00 > m22) {
    const s = 2 * Math.sqrt(1 + m00 - m11 - m22);
    q = [0.25 * s, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s];
  } else if (m11 > m22) {
    const s = 2 * Math.sqrt(1 + m11 - m00 - m22);
    q = [(m01 + m10) / s, 0.25 * s, (m12 + m21) / s, (m02 - m20) / s];
  } else {
    const s = 2 * Math.sqrt(1 + m22 - m00 - m11);
    q = [(m02 + m20) / s, (m12 + m21) / s, 0.25 * s, (m10 - m01) / s];
  }
  return qNorm(q);
}

/** Rotation part of a column-major 4×4 matrix (scale removed). */
export function qFromMat4(m: ArrayLike<number>): Quat {
  const x = norm([m[0], m[1], m[2]]);
  const y = norm([m[4], m[5], m[6]]);
  const z = norm([m[8], m[9], m[10]]);
  return qFromBasis(x, y, z);
}

/** Orthonormal frame from a primary axis and a hint for the second one. */
export function frame(primary: Vec3, secondaryHint: Vec3): [Vec3, Vec3, Vec3] {
  const a = norm(primary);
  const b = norm(sub(secondaryHint, scale(a, dot(secondaryHint, a))));
  return [a, b, cross(a, b)];
}

/**
 * Rotation taking the frame (a0, b0) onto (a1, b1), where each pair is a
 * primary axis plus a secondary hint. Used to give a bone its twist too.
 */
export function qFromFrames(a0: Vec3, b0: Vec3, a1: Vec3, b1: Vec3): Quat {
  const [x0, y0, z0] = frame(a0, b0);
  const [x1, y1, z1] = frame(a1, b1);
  const r0 = qFromBasis(x0, y0, z0);
  const r1 = qFromBasis(x1, y1, z1);
  return qNorm(qMul(r1, qConj(r0)));
}

export function qSlerp(a: Quat, b: Quat, t: number): Quat {
  let bx = b[0], by = b[1], bz = b[2], bw = b[3];
  let cos = qDot(a, b);
  if (cos < 0) {
    cos = -cos;
    bx = -bx;
    by = -by;
    bz = -bz;
    bw = -bw;
  }
  if (cos > 0.9995) {
    return qNorm([
      a[0] + (bx - a[0]) * t,
      a[1] + (by - a[1]) * t,
      a[2] + (bz - a[2]) * t,
      a[3] + (bw - a[3]) * t,
    ]);
  }
  const theta = Math.acos(clamp(cos, -1, 1));
  const sin = Math.sin(theta);
  const wa = Math.sin((1 - t) * theta) / sin;
  const wb = Math.sin(t * theta) / sin;
  return [a[0] * wa + bx * wb, a[1] * wa + by * wb, a[2] * wa + bz * wb, a[3] * wa + bw * wb];
}

/** Rotation angle of a unit quaternion, in [0, π]. */
export function qAngle(q: Quat): number {
  return 2 * Math.acos(clamp(Math.abs(q[3]), 0, 1));
}

/**
 * Swing-twist split around `axis` (unit): q = swing * twist, where `twist`
 * turns around the axis and `swing` moves the axis itself.
 */
export function swingTwist(q: Quat, axis: Vec3): { swing: Quat; twist: Quat } {
  const p = dot([q[0], q[1], q[2]], axis);
  let twist: Quat = [axis[0] * p, axis[1] * p, axis[2] * p, q[3]];
  const l = Math.hypot(twist[0], twist[1], twist[2], twist[3]);
  twist = l < 1e-9 ? [0, 0, 0, 1] : [twist[0] / l, twist[1] / l, twist[2] / l, twist[3] / l];
  const swing = qMul(q, qConj(twist));
  return { swing: qNorm(swing), twist };
}

/** Scales a rotation's angle down to at most `max` radians. */
export function limitAngle(q: Quat, max: number): Quat {
  const ang = qAngle(q);
  if (ang <= max || ang < 1e-9) return q;
  return qSlerp(QI, q, max / ang);
}

/**
 * Joint limit: the swing (how far the bone axis leaves rest) is capped at
 * `maxSwing`, and the twist around the bone axis at `maxTwist`.
 */
export function limitSwingTwist(q: Quat, axis: Vec3, maxSwing: number, maxTwist: number): Quat {
  const { swing, twist } = swingTwist(q, axis);
  return qNorm(qMul(limitAngle(swing, maxSwing), limitAngle(twist, maxTwist)));
}

/** Intrinsic XYZ Euler angles (radians) to a quaternion. */
export function qFromEuler(x: number, y: number, z: number): Quat {
  const c1 = Math.cos(x / 2), c2 = Math.cos(y / 2), c3 = Math.cos(z / 2);
  const s1 = Math.sin(x / 2), s2 = Math.sin(y / 2), s3 = Math.sin(z / 2);
  return [
    s1 * c2 * c3 + c1 * s2 * s3,
    c1 * s2 * c3 - s1 * c2 * s3,
    c1 * c2 * s3 + s1 * s2 * c3,
    c1 * c2 * c3 - s1 * s2 * s3,
  ];
}

/** Keeps `q` in the same hemisphere as `ref`, so filters do not flip. */
export function qAlign(q: Quat, ref: Quat): Quat {
  return qDot(q, ref) < 0 ? [-q[0], -q[1], -q[2], -q[3]] : q;
}

/** Mirror across the YZ plane (left becomes right). */
export const qMirrorX = (q: Quat): Quat => [q[0], -q[1], -q[2], q[3]];
