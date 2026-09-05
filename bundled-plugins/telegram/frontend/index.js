var el = Object.defineProperty;
var Gn = (e) => {
  throw TypeError(e);
};
var tl = (e, r, t) => r in e ? el(e, r, { enumerable: !0, configurable: !0, writable: !0, value: t }) : e[r] = t;
var Tt = (e, r, t) => tl(e, typeof r != "symbol" ? r + "" : r, t), fn = (e, r, t) => r.has(e) || Gn("Cannot " + t);
var p = (e, r, t) => (fn(e, r, "read from private field"), t ? t.call(e) : r.get(e)), ee = (e, r, t) => r.has(e) ? Gn("Cannot add the same private member more than once") : r instanceof WeakSet ? r.add(e) : r.set(e, t), Ft = (e, r, t, n) => (fn(e, r, "write to private field"), n ? n.call(e, t) : r.set(e, t), t), Re = (e, r, t) => (fn(e, r, "access private method"), t);
import { pluginInvoke as _e } from "$lib/plugin-invoke";
import { listen as Yn } from "@tauri-apps/api/event";
import { open as Qn } from "@tauri-apps/plugin-dialog";
import { showToast as Bt } from "$lib/stores/toast-store.svelte";
import { getSettings as rl } from "$lib/stores/settings-store.svelte";
import { onBatchFileStatus as Xn } from "$lib/stores/download-listener";
import nl from "$components/hints/ContextHint.svelte";
import { t as al } from "$lib/i18n";
const sl = "5";
var oa;
typeof window < "u" && ((oa = window.__svelte ?? (window.__svelte = {})).v ?? (oa.v = /* @__PURE__ */ new Set())).add(sl);
const ll = 1, il = 2, ua = 4, ol = 8, fl = 16, ca = 1, ul = 2, he = Symbol(), va = "http://www.w3.org/1999/xhtml", cl = "http://www.w3.org/2000/svg", vl = "http://www.w3.org/1998/Math/MathML", dl = !1;
var da = Array.isArray, _l = Array.prototype.indexOf, lr = Array.prototype.includes, An = Array.from, _a = Object.defineProperty, un = Object.getOwnPropertyDescriptor, ha = Object.getOwnPropertyDescriptors, hl = Object.prototype, pl = Array.prototype, Cn = Object.getPrototypeOf;
const jr = () => {
};
function gl(e) {
  for (var r = 0; r < e.length; r++)
    e[r]();
}
function pa() {
  var e, r, t = new Promise((n, a) => {
    e = n, r = a;
  });
  return { promise: t, resolve: e, reject: r };
}
const ue = 2, ir = 4, Gr = 8, ga = 1 << 24, kt = 16, st = 32, or = 64, wl = 128, Qe = 512, le = 1024, ge = 2048, _t = 4096, Ae = 8192, Xe = 16384, Xt = 32768, mn = 1 << 25, mr = 65536, bn = 1 << 17, ml = 1 << 18, Yr = 1 << 19, bl = 1 << 20, dt = 1 << 25, Yt = 65536, yn = 1 << 21, Qr = 1 << 22, It = 1 << 23, rr = Symbol("$state"), yl = Symbol(""), bt = new class extends Error {
  constructor() {
    super(...arguments);
    Tt(this, "name", "StaleReactionError");
    Tt(this, "message", "The reaction that called `getAbortSignal()` was re-run or destroyed");
  }
}();
function ql() {
  throw new Error("https://svelte.dev/e/async_derived_orphan");
}
function xl(e, r, t) {
  throw new Error("https://svelte.dev/e/each_key_duplicate");
}
function kl(e) {
  throw new Error("https://svelte.dev/e/effect_in_teardown");
}
function El() {
  throw new Error("https://svelte.dev/e/effect_in_unowned_derived");
}
function Ml(e) {
  throw new Error("https://svelte.dev/e/effect_orphan");
}
function Tl() {
  throw new Error("https://svelte.dev/e/effect_update_depth_exceeded");
}
function Sl() {
  throw new Error("https://svelte.dev/e/state_descriptors_fixed");
}
function Al() {
  throw new Error("https://svelte.dev/e/state_prototype_fixed");
}
function Cl() {
  throw new Error("https://svelte.dev/e/state_unsafe_mutation");
}
function wa(e) {
  return e === this.v;
}
function ma(e, r) {
  return e != e ? r == r : e !== r || e !== null && typeof e == "object" || typeof e == "function";
}
function ba(e) {
  return !ma(e, this.v);
}
let Il = !1, lt = null;
function Hr(e) {
  lt = e;
}
function Dl(e, r = !1, t) {
  lt = {
    p: lt,
    i: !1,
    c: null,
    e: null,
    s: e,
    x: null,
    r: (
      /** @type {Effect} */
      V
    ),
    l: null
  };
}
function Nl(e) {
  var r = (
    /** @type {ComponentContext} */
    lt
  ), t = r.e;
  if (t !== null) {
    r.e = null;
    for (var n of t)
      Ba(n);
  }
  return r.i = !0, lt = r.p, /** @type {T} */
  {};
}
function ya() {
  return !0;
}
let Ht = [];
function qa() {
  var e = Ht;
  Ht = [], gl(e);
}
function zr(e) {
  if (Ht.length === 0 && !wr) {
    var r = Ht;
    queueMicrotask(() => {
      r === Ht && qa();
    });
  }
  Ht.push(e);
}
function Ll() {
  for (; Ht.length > 0; )
    qa();
}
function Rl(e) {
  var r = V;
  if (r === null)
    return O.f |= It, e;
  if ((r.f & Xt) === 0 && (r.f & ir) === 0)
    throw e;
  Ur(e, r);
}
function Ur(e, r) {
  for (; r !== null; ) {
    if ((r.f & wl) !== 0) {
      if ((r.f & Xt) === 0)
        throw e;
      try {
        r.b.error(e);
        return;
      } catch (t) {
        e = t;
      }
    }
    r = r.parent;
  }
  throw e;
}
const Pl = -7169;
function te(e, r) {
  e.f = e.f & Pl | r;
}
function In(e) {
  (e.f & Qe) !== 0 || e.deps === null ? te(e, le) : te(e, _t);
}
function xa(e) {
  if (e !== null)
    for (const r of e)
      (r.f & ue) === 0 || (r.f & Yt) === 0 || (r.f ^= Yt, xa(
        /** @type {Derived} */
        r.deps
      ));
}
function Ol(e, r, t) {
  (e.f & ge) !== 0 ? r.add(e) : (e.f & _t) !== 0 && t.add(e), xa(e.deps), te(e, le);
}
function ka(e, r, t) {
  if (e == null)
    return r(void 0), jr;
  const n = Zr(
    () => e.subscribe(
      r,
      // @ts-expect-error
      t
    )
  );
  return n.unsubscribe ? () => n.unsubscribe() : n;
}
function Fl(e) {
  let r;
  return ka(e, (t) => r = t)(), r;
}
let qn = Symbol();
function Bl(e, r, t) {
  const n = t[r] ?? (t[r] = {
    store: null,
    source: /* @__PURE__ */ La(void 0),
    unsubscribe: jr
  });
  if (n.store !== e && !(qn in t))
    if (n.unsubscribe(), n.store = e ?? null, e == null)
      n.source.v = void 0, n.unsubscribe = jr;
    else {
      var a = !0;
      n.unsubscribe = ka(e, (l) => {
        a ? n.source.v = l : c(n.source, l);
      }), a = !1;
    }
  return e && qn in t ? Fl(e) : s(n.source);
}
function jl() {
  const e = {};
  function r() {
    Pn(() => {
      for (var t in e)
        e[t].unsubscribe();
      _a(e, qn, {
        enumerable: !1,
        value: !0
      });
    });
  }
  return [e, r];
}
const St = /* @__PURE__ */ new Set();
let D = null, pe = null, xn = null, wr = !1, cn = !1, tr = null, Pr = null;
var Zn = 0;
let Hl = 1;
var nr, ar, yt, ut, Er, Oe, Mr, Ct, qt, ct, sr, Vt, ne, Or, Ea, Fr, kn, En, Ma;
const Kr = class Kr {
  constructor() {
    ee(this, ne);
    Tt(this, "id", Hl++);
    /**
     * The current values of any signals that are updated in this batch.
     * Tuple format: [value, is_derived] (note: is_derived is false for deriveds, too, if they were overridden via assignment)
     * They keys of this map are identical to `this.#previous`
     * @type {Map<Value, [any, boolean]>}
     */
    Tt(this, "current", /* @__PURE__ */ new Map());
    /**
     * The values of any signals (sources and deriveds) that are updated in this batch _before_ those updates took place.
     * They keys of this map are identical to `this.#current`
     * @type {Map<Value, any>}
     */
    Tt(this, "previous", /* @__PURE__ */ new Map());
    /**
     * When the batch is committed (and the DOM is updated), we need to remove old branches
     * and append new ones by calling the functions added inside (if/each/key/etc) blocks
     * @type {Set<(batch: Batch) => void>}
     */
    ee(this, nr, /* @__PURE__ */ new Set());
    /**
     * If a fork is discarded, we need to destroy any effects that are no longer needed
     * @type {Set<(batch: Batch) => void>}
     */
    ee(this, ar, /* @__PURE__ */ new Set());
    /**
     * Async effects that are currently in flight
     * @type {Map<Effect, number>}
     */
    ee(this, yt, /* @__PURE__ */ new Map());
    /**
     * Async effects that are currently in flight, _not_ inside a pending boundary
     * @type {Map<Effect, number>}
     */
    ee(this, ut, /* @__PURE__ */ new Map());
    /**
     * A deferred that resolves when the batch is committed, used with `settled()`
     * TODO replace with Promise.withResolvers once supported widely enough
     * @type {{ promise: Promise<void>, resolve: (value?: any) => void, reject: (reason: unknown) => void } | null}
     */
    ee(this, Er, null);
    /**
     * The root effects that need to be flushed
     * @type {Effect[]}
     */
    ee(this, Oe, []);
    /**
     * Effects created while this batch was active.
     * @type {Effect[]}
     */
    ee(this, Mr, []);
    /**
     * Deferred effects (which run after async work has completed) that are DIRTY
     * @type {Set<Effect>}
     */
    ee(this, Ct, /* @__PURE__ */ new Set());
    /**
     * Deferred effects that are MAYBE_DIRTY
     * @type {Set<Effect>}
     */
    ee(this, qt, /* @__PURE__ */ new Set());
    /**
     * A map of branches that still exist, but will be destroyed when this batch
     * is committed — we skip over these during `process`.
     * The value contains child effects that were dirty/maybe_dirty before being reset,
     * so they can be rescheduled if the branch survives.
     * @type {Map<Effect, { d: Effect[], m: Effect[] }>}
     */
    ee(this, ct, /* @__PURE__ */ new Map());
    Tt(this, "is_fork", !1);
    ee(this, sr, !1);
    /** @type {Set<Batch>} */
    ee(this, Vt, /* @__PURE__ */ new Set());
  }
  /**
   * Add an effect to the #skipped_branches map and reset its children
   * @param {Effect} effect
   */
  skip_effect(r) {
    p(this, ct).has(r) || p(this, ct).set(r, { d: [], m: [] });
  }
  /**
   * Remove an effect from the #skipped_branches map and reschedule
   * any tracked dirty/maybe_dirty child effects
   * @param {Effect} effect
   */
  unskip_effect(r) {
    var t = p(this, ct).get(r);
    if (t) {
      p(this, ct).delete(r);
      for (var n of t.d)
        te(n, ge), this.schedule(n);
      for (n of t.m)
        te(n, _t), this.schedule(n);
    }
  }
  /**
   * Associate a change to a given source with the current
   * batch, noting its previous and current values
   * @param {Value} source
   * @param {any} old_value
   * @param {boolean} [is_derived]
   */
  capture(r, t, n = !1) {
    t !== he && !this.previous.has(r) && this.previous.set(r, t), (r.f & It) === 0 && (this.current.set(r, [r.v, n]), pe == null || pe.set(r, r.v));
  }
  activate() {
    D = this;
  }
  deactivate() {
    D = null, pe = null;
  }
  flush() {
    try {
      cn = !0, D = this, Re(this, ne, Fr).call(this);
    } finally {
      Zn = 0, xn = null, tr = null, Pr = null, cn = !1, D = null, pe = null, Dt.clear();
    }
  }
  discard() {
    for (const r of p(this, ar)) r(this);
    p(this, ar).clear(), St.delete(this);
  }
  /**
   * @param {Effect} effect
   */
  register_created_effect(r) {
    p(this, Mr).push(r);
  }
  /**
   * @param {boolean} blocking
   * @param {Effect} effect
   */
  increment(r, t) {
    let n = p(this, yt).get(t) ?? 0;
    if (p(this, yt).set(t, n + 1), r) {
      let a = p(this, ut).get(t) ?? 0;
      p(this, ut).set(t, a + 1);
    }
  }
  /**
   * @param {boolean} blocking
   * @param {Effect} effect
   * @param {boolean} skip - whether to skip updates (because this is triggered by a stale reaction)
   */
  decrement(r, t, n) {
    let a = p(this, yt).get(t) ?? 0;
    if (a === 1 ? p(this, yt).delete(t) : p(this, yt).set(t, a - 1), r) {
      let l = p(this, ut).get(t) ?? 0;
      l === 1 ? p(this, ut).delete(t) : p(this, ut).set(t, l - 1);
    }
    p(this, sr) || n || (Ft(this, sr, !0), zr(() => {
      Ft(this, sr, !1), this.flush();
    }));
  }
  /**
   * @param {Set<Effect>} dirty_effects
   * @param {Set<Effect>} maybe_dirty_effects
   */
  transfer_effects(r, t) {
    for (const n of r)
      p(this, Ct).add(n);
    for (const n of t)
      p(this, qt).add(n);
    r.clear(), t.clear();
  }
  /** @param {(batch: Batch) => void} fn */
  oncommit(r) {
    p(this, nr).add(r);
  }
  /** @param {(batch: Batch) => void} fn */
  ondiscard(r) {
    p(this, ar).add(r);
  }
  settled() {
    return (p(this, Er) ?? Ft(this, Er, pa())).promise;
  }
  static ensure() {
    if (D === null) {
      const r = D = new Kr();
      cn || (St.add(D), wr || zr(() => {
        D === r && r.flush();
      }));
    }
    return D;
  }
  apply() {
    {
      pe = null;
      return;
    }
  }
  /**
   *
   * @param {Effect} effect
   */
  schedule(r) {
    var a;
    if (xn = r, (a = r.b) != null && a.is_pending && (r.f & (ir | Gr | ga)) !== 0 && (r.f & Xt) === 0) {
      r.b.defer_effect(r);
      return;
    }
    for (var t = r; t.parent !== null; ) {
      t = t.parent;
      var n = t.f;
      if (tr !== null && t === V && (O === null || (O.f & ue) === 0))
        return;
      if ((n & (or | st)) !== 0) {
        if ((n & le) === 0)
          return;
        t.f ^= le;
      }
    }
    p(this, Oe).push(t);
  }
};
nr = new WeakMap(), ar = new WeakMap(), yt = new WeakMap(), ut = new WeakMap(), Er = new WeakMap(), Oe = new WeakMap(), Mr = new WeakMap(), Ct = new WeakMap(), qt = new WeakMap(), ct = new WeakMap(), sr = new WeakMap(), Vt = new WeakMap(), ne = new WeakSet(), Or = function() {
  return this.is_fork || p(this, ut).size > 0;
}, Ea = function() {
  for (const n of p(this, Vt))
    for (const a of p(n, ut).keys()) {
      for (var r = !1, t = a; t.parent !== null; ) {
        if (p(this, ct).has(t)) {
          r = !0;
          break;
        }
        t = t.parent;
      }
      if (!r)
        return !0;
    }
  return !1;
}, Fr = function() {
  var u, o;
  if (Zn++ > 1e3 && (St.delete(this), Ul()), !Re(this, ne, Or).call(this)) {
    for (const v of p(this, Ct))
      p(this, qt).delete(v), te(v, ge), this.schedule(v);
    for (const v of p(this, qt))
      te(v, _t), this.schedule(v);
  }
  const r = p(this, Oe);
  Ft(this, Oe, []), this.apply();
  var t = tr = [], n = [], a = Pr = [];
  for (const v of r)
    try {
      Re(this, ne, kn).call(this, v, t, n);
    } catch (d) {
      throw Aa(v), d;
    }
  if (D = null, a.length > 0) {
    var l = Kr.ensure();
    for (const v of a)
      l.schedule(v);
  }
  if (tr = null, Pr = null, Re(this, ne, Or).call(this) || Re(this, ne, Ea).call(this)) {
    Re(this, ne, En).call(this, n), Re(this, ne, En).call(this, t);
    for (const [v, d] of p(this, ct))
      Sa(v, d);
  } else {
    p(this, yt).size === 0 && St.delete(this), p(this, Ct).clear(), p(this, qt).clear();
    for (const v of p(this, nr)) v(this);
    p(this, nr).clear(), Jn(n), Jn(t), (u = p(this, Er)) == null || u.resolve();
  }
  var i = (
    /** @type {Batch | null} */
    /** @type {unknown} */
    D
  );
  if (p(this, Oe).length > 0) {
    const v = i ?? (i = this);
    p(v, Oe).push(...p(this, Oe).filter((d) => !p(v, Oe).includes(d)));
  }
  i !== null && (St.add(i), Re(o = i, ne, Fr).call(o)), St.has(this) || Re(this, ne, Ma).call(this);
}, /**
 * Traverse the effect tree, executing effects or stashing
 * them for later execution as appropriate
 * @param {Effect} root
 * @param {Effect[]} effects
 * @param {Effect[]} render_effects
 */
kn = function(r, t, n) {
  r.f ^= le;
  for (var a = r.first; a !== null; ) {
    var l = a.f, i = (l & (st | or)) !== 0, u = i && (l & le) !== 0, o = u || (l & Ae) !== 0 || p(this, ct).has(a);
    if (!o && a.fn !== null) {
      i ? a.f ^= le : (l & ir) !== 0 ? t.push(a) : Cr(a) && ((l & kt) !== 0 && p(this, qt).add(a), fr(a));
      var v = a.first;
      if (v !== null) {
        a = v;
        continue;
      }
    }
    for (; a !== null; ) {
      var d = a.next;
      if (d !== null) {
        a = d;
        break;
      }
      a = a.parent;
    }
  }
}, /**
 * @param {Effect[]} effects
 */
En = function(r) {
  for (var t = 0; t < r.length; t += 1)
    Ol(r[t], p(this, Ct), p(this, qt));
}, Ma = function() {
  var d, m, _;
  for (const y of St) {
    var r = y.id < this.id, t = [];
    for (const [b, [z, w]] of this.current) {
      if (y.current.has(b)) {
        var n = (
          /** @type {[any, boolean]} */
          y.current.get(b)[0]
        );
        if (r && z !== n)
          y.current.set(b, [z, w]);
        else
          continue;
      }
      t.push(b);
    }
    var a = [...y.current.keys()].filter((b) => !this.current.has(b));
    if (a.length === 0)
      r && y.discard();
    else if (t.length > 0) {
      y.activate();
      var l = /* @__PURE__ */ new Set(), i = /* @__PURE__ */ new Map();
      for (var u of t)
        Ta(u, a, l, i);
      i = /* @__PURE__ */ new Map();
      var o = [...y.current.keys()].filter(
        (b) => this.current.has(b) ? (
          /** @type {[any, boolean]} */
          this.current.get(b)[0] !== b
        ) : !0
      );
      for (const b of p(this, Mr))
        (b.f & (Xe | Ae | bn)) === 0 && Dn(b, o, i) && ((b.f & (Qr | kt)) !== 0 ? (te(b, ge), y.schedule(b)) : p(y, Ct).add(b));
      if (p(y, Oe).length > 0) {
        y.apply();
        for (var v of p(y, Oe))
          Re(d = y, ne, kn).call(d, v, [], []);
        Ft(y, Oe, []);
      }
      y.deactivate();
    }
  }
  for (const y of St)
    p(y, Vt).has(this) && (p(y, Vt).delete(this), p(y, Vt).size === 0 && !Re(m = y, ne, Or).call(m) && (y.activate(), Re(_ = y, ne, Fr).call(_)));
};
let br = Kr;
function zl(e) {
  var r = wr;
  wr = !0;
  try {
    for (var t; ; ) {
      if (Ll(), D === null)
        return (
          /** @type {T} */
          t
        );
      D.flush();
    }
  } finally {
    wr = r;
  }
}
function Ul() {
  try {
    Tl();
  } catch (e) {
    Ur(e, xn);
  }
}
let rt = null;
function Jn(e) {
  var r = e.length;
  if (r !== 0) {
    for (var t = 0; t < r; ) {
      var n = e[t++];
      if ((n.f & (Xe | Ae)) === 0 && Cr(n) && (rt = /* @__PURE__ */ new Set(), fr(n), n.deps === null && n.first === null && n.nodes === null && n.teardown === null && n.ac === null && Va(n), (rt == null ? void 0 : rt.size) > 0)) {
        Dt.clear();
        for (const a of rt) {
          if ((a.f & (Xe | Ae)) !== 0) continue;
          const l = [a];
          let i = a.parent;
          for (; i !== null; )
            rt.has(i) && (rt.delete(i), l.push(i)), i = i.parent;
          for (let u = l.length - 1; u >= 0; u--) {
            const o = l[u];
            (o.f & (Xe | Ae)) === 0 && fr(o);
          }
        }
        rt.clear();
      }
    }
    rt = null;
  }
}
function Ta(e, r, t, n) {
  if (!t.has(e) && (t.add(e), e.reactions !== null))
    for (const a of e.reactions) {
      const l = a.f;
      (l & ue) !== 0 ? Ta(
        /** @type {Derived} */
        a,
        r,
        t,
        n
      ) : (l & (Qr | kt)) !== 0 && (l & ge) === 0 && Dn(a, r, n) && (te(a, ge), Nn(
        /** @type {Effect} */
        a
      ));
    }
}
function Dn(e, r, t) {
  const n = t.get(e);
  if (n !== void 0) return n;
  if (e.deps !== null)
    for (const a of e.deps) {
      if (lr.call(r, a))
        return !0;
      if ((a.f & ue) !== 0 && Dn(
        /** @type {Derived} */
        a,
        r,
        t
      ))
        return t.set(
          /** @type {Derived} */
          a,
          !0
        ), !0;
    }
  return t.set(e, !1), !1;
}
function Nn(e) {
  D.schedule(e);
}
function Sa(e, r) {
  if (!((e.f & st) !== 0 && (e.f & le) !== 0)) {
    (e.f & ge) !== 0 ? r.d.push(e) : (e.f & _t) !== 0 && r.m.push(e), te(e, le);
    for (var t = e.first; t !== null; )
      Sa(t, r), t = t.next;
  }
}
function Aa(e) {
  te(e, le);
  for (var r = e.first; r !== null; )
    Aa(r), r = r.next;
}
function Vl(e, r, t, n) {
  const a = Ln;
  var l = e.filter((_) => !_.settled);
  if (t.length === 0 && l.length === 0) {
    n(r.map(a));
    return;
  }
  var i = (
    /** @type {Effect} */
    V
  ), u = Kl(), o = l.length === 1 ? l[0].promise : l.length > 1 ? Promise.all(l.map((_) => _.promise)) : null;
  function v(_) {
    u();
    try {
      n(_);
    } catch (y) {
      (i.f & Xe) === 0 && Ur(y, i);
    }
    Vr();
  }
  if (t.length === 0) {
    o.then(() => v(r.map(a)));
    return;
  }
  var d = Ca();
  function m() {
    Promise.all(t.map((_) => /* @__PURE__ */ Wl(_))).then((_) => v([...r.map(a), ..._])).catch((_) => Ur(_, i)).finally(() => d());
  }
  o ? o.then(() => {
    u(), m(), Vr();
  }) : m();
}
function Kl() {
  var e = (
    /** @type {Effect} */
    V
  ), r = O, t = lt, n = (
    /** @type {Batch} */
    D
  );
  return function(l = !0) {
    Nt(e), ht(r), Hr(t), l && (e.f & Xe) === 0 && (n == null || n.activate(), n == null || n.apply());
  };
}
function Vr(e = !0) {
  Nt(null), ht(null), Hr(null), e && (D == null || D.deactivate());
}
function Ca() {
  var e = (
    /** @type {Effect} */
    V
  ), r = (
    /** @type {Boundary} */
    e.b
  ), t = (
    /** @type {Batch} */
    D
  ), n = r.is_rendered();
  return r.update_pending_count(1, t), t.increment(n, e), (a = !1) => {
    r.update_pending_count(-1, t), t.decrement(n, e, a);
  };
}
// @__NO_SIDE_EFFECTS__
function Ln(e) {
  var r = ue | ge, t = O !== null && (O.f & ue) !== 0 ? (
    /** @type {Derived} */
    O
  ) : null;
  return V !== null && (V.f |= Yr), {
    ctx: lt,
    deps: null,
    effects: null,
    equals: wa,
    f: r,
    fn: e,
    reactions: null,
    rv: 0,
    v: (
      /** @type {V} */
      he
    ),
    wv: 0,
    parent: t ?? V,
    ac: null
  };
}
// @__NO_SIDE_EFFECTS__
function Wl(e, r, t) {
  let n = (
    /** @type {Effect | null} */
    V
  );
  n === null && ql();
  var a = (
    /** @type {Promise<V>} */
    /** @type {unknown} */
    void 0
  ), l = yr(
    /** @type {V} */
    he
  ), i = !O, u = /* @__PURE__ */ new Map();
  return ii(() => {
    var y;
    var o = (
      /** @type {Effect} */
      V
    ), v = pa();
    a = v.promise;
    try {
      Promise.resolve(e()).then(v.resolve, v.reject).finally(Vr);
    } catch (b) {
      v.reject(b), Vr();
    }
    var d = (
      /** @type {Batch} */
      D
    );
    if (i) {
      if ((o.f & Xt) !== 0)
        var m = Ca();
      if (
        /** @type {Boundary} */
        n.b.is_rendered()
      )
        (y = u.get(d)) == null || y.reject(bt), u.delete(d);
      else {
        for (const b of u.values())
          b.reject(bt);
        u.clear();
      }
      u.set(d, v);
    }
    const _ = (b, z = void 0) => {
      if (m) {
        var w = z === bt;
        m(w);
      }
      if (!(z === bt || (o.f & Xe) !== 0)) {
        if (d.activate(), z)
          l.f |= It, qr(l, z);
        else {
          (l.f & It) !== 0 && (l.f ^= It), qr(l, b);
          for (const [F, W] of u) {
            if (u.delete(F), F === d) break;
            W.reject(bt);
          }
        }
        d.deactivate();
      }
    };
    v.promise.then(_, (b) => _(null, b || "unknown"));
  }), Pn(() => {
    for (const o of u.values())
      o.reject(bt);
  }), new Promise((o) => {
    function v(d) {
      function m() {
        d === a ? o(l) : v(a);
      }
      d.then(m, m);
    }
    v(a);
  });
}
// @__NO_SIDE_EFFECTS__
function ft(e) {
  const r = /* @__PURE__ */ Ln(e);
  return Ya(r), r;
}
// @__NO_SIDE_EFFECTS__
function Gl(e) {
  const r = /* @__PURE__ */ Ln(e);
  return r.equals = ba, r;
}
function Yl(e) {
  var r = e.effects;
  if (r !== null) {
    e.effects = null;
    for (var t = 0; t < r.length; t += 1)
      xt(
        /** @type {Effect} */
        r[t]
      );
  }
}
function Ql(e) {
  for (var r = e.parent; r !== null; ) {
    if ((r.f & ue) === 0)
      return (r.f & Xe) === 0 ? (
        /** @type {Effect} */
        r
      ) : null;
    r = r.parent;
  }
  return null;
}
function Rn(e) {
  var r, t = V;
  Nt(Ql(e));
  try {
    e.f &= ~Yt, Yl(e), r = Ja(e);
  } finally {
    Nt(t);
  }
  return r;
}
function Ia(e) {
  var r = e.v, t = Rn(e);
  if (!e.equals(t) && (e.wv = Xa(), (!(D != null && D.is_fork) || e.deps === null) && (e.v = t, D == null || D.capture(e, r, !0), e.deps === null))) {
    te(e, le);
    return;
  }
  Qt || (pe !== null ? (Fa() || D != null && D.is_fork) && pe.set(e, t) : In(e));
}
function Xl(e) {
  var r, t;
  if (e.effects !== null)
    for (const n of e.effects)
      (n.teardown || n.ac) && ((r = n.teardown) == null || r.call(n), (t = n.ac) == null || t.abort(bt), n.teardown = jr, n.ac = null, kr(n, 0), Fn(n));
}
function Da(e) {
  if (e.effects !== null)
    for (const r of e.effects)
      r.teardown && fr(r);
}
let Mn = /* @__PURE__ */ new Set();
const Dt = /* @__PURE__ */ new Map();
let Na = !1;
function yr(e, r) {
  var t = {
    f: 0,
    // TODO ideally we could skip this altogether, but it causes type errors
    v: e,
    reactions: null,
    equals: wa,
    rv: 0,
    wv: 0
  };
  return t;
}
// @__NO_SIDE_EFFECTS__
function A(e, r) {
  const t = yr(e);
  return Ya(t), t;
}
// @__NO_SIDE_EFFECTS__
function La(e, r = !1, t = !0) {
  const n = yr(e);
  return r || (n.equals = ba), n;
}
function c(e, r, t = !1) {
  O !== null && // since we are untracking the function inside `$inspect.with` we need to add this check
  // to ensure we error if state is set inside an inspect effect
  (!at || (O.f & bn) !== 0) && ya() && (O.f & (ue | kt | Qr | bn)) !== 0 && (Ze === null || !lr.call(Ze, e)) && Cl();
  let n = t ? Be(r) : r;
  return qr(e, n, Pr);
}
function qr(e, r, t = null) {
  if (!e.equals(r)) {
    var n = e.v;
    Qt ? Dt.set(e, r) : Dt.set(e, n), e.v = r;
    var a = br.ensure();
    if (a.capture(e, n), (e.f & ue) !== 0) {
      const l = (
        /** @type {Derived} */
        e
      );
      (e.f & ge) !== 0 && Rn(l), pe === null && In(l);
    }
    e.wv = Xa(), Ra(e, ge, t), V !== null && (V.f & le) !== 0 && (V.f & (st | or)) === 0 && (Ye === null ? fi([e]) : Ye.push(e)), !a.is_fork && Mn.size > 0 && !Na && Zl();
  }
  return r;
}
function Zl() {
  Na = !1;
  for (const e of Mn)
    (e.f & le) !== 0 && te(e, _t), Cr(e) && fr(e);
  Mn.clear();
}
function vn(e) {
  c(e, e.v + 1);
}
function Ra(e, r, t) {
  var n = e.reactions;
  if (n !== null)
    for (var a = n.length, l = 0; l < a; l++) {
      var i = n[l], u = i.f, o = (u & ge) === 0;
      if (o && te(i, r), (u & ue) !== 0) {
        var v = (
          /** @type {Derived} */
          i
        );
        pe == null || pe.delete(v), (u & Yt) === 0 && (u & Qe && (i.f |= Yt), Ra(v, _t, t));
      } else if (o) {
        var d = (
          /** @type {Effect} */
          i
        );
        (u & kt) !== 0 && rt !== null && rt.add(d), t !== null ? t.push(d) : Nn(d);
      }
    }
}
function Be(e) {
  if (typeof e != "object" || e === null || rr in e)
    return e;
  const r = Cn(e);
  if (r !== hl && r !== pl)
    return e;
  var t = /* @__PURE__ */ new Map(), n = da(e), a = /* @__PURE__ */ A(0), l = Gt, i = (u) => {
    if (Gt === l)
      return u();
    var o = O, v = Gt;
    ht(null), ta(l);
    var d = u();
    return ht(o), ta(v), d;
  };
  return n && t.set("length", /* @__PURE__ */ A(
    /** @type {any[]} */
    e.length
  )), new Proxy(
    /** @type {any} */
    e,
    {
      defineProperty(u, o, v) {
        (!("value" in v) || v.configurable === !1 || v.enumerable === !1 || v.writable === !1) && Sl();
        var d = t.get(o);
        return d === void 0 ? i(() => {
          var m = /* @__PURE__ */ A(v.value);
          return t.set(o, m), m;
        }) : c(d, v.value, !0), !0;
      },
      deleteProperty(u, o) {
        var v = t.get(o);
        if (v === void 0) {
          if (o in u) {
            const d = i(() => /* @__PURE__ */ A(he));
            t.set(o, d), vn(a);
          }
        } else
          c(v, he), vn(a);
        return !0;
      },
      get(u, o, v) {
        var y;
        if (o === rr)
          return e;
        var d = t.get(o), m = o in u;
        if (d === void 0 && (!m || (y = un(u, o)) != null && y.writable) && (d = i(() => {
          var b = Be(m ? u[o] : he), z = /* @__PURE__ */ A(b);
          return z;
        }), t.set(o, d)), d !== void 0) {
          var _ = s(d);
          return _ === he ? void 0 : _;
        }
        return Reflect.get(u, o, v);
      },
      getOwnPropertyDescriptor(u, o) {
        var v = Reflect.getOwnPropertyDescriptor(u, o);
        if (v && "value" in v) {
          var d = t.get(o);
          d && (v.value = s(d));
        } else if (v === void 0) {
          var m = t.get(o), _ = m == null ? void 0 : m.v;
          if (m !== void 0 && _ !== he)
            return {
              enumerable: !0,
              configurable: !0,
              value: _,
              writable: !0
            };
        }
        return v;
      },
      has(u, o) {
        var _;
        if (o === rr)
          return !0;
        var v = t.get(o), d = v !== void 0 && v.v !== he || Reflect.has(u, o);
        if (v !== void 0 || V !== null && (!d || (_ = un(u, o)) != null && _.writable)) {
          v === void 0 && (v = i(() => {
            var y = d ? Be(u[o]) : he, b = /* @__PURE__ */ A(y);
            return b;
          }), t.set(o, v));
          var m = s(v);
          if (m === he)
            return !1;
        }
        return d;
      },
      set(u, o, v, d) {
        var Y;
        var m = t.get(o), _ = o in u;
        if (n && o === "length")
          for (var y = v; y < /** @type {Source<number>} */
          m.v; y += 1) {
            var b = t.get(y + "");
            b !== void 0 ? c(b, he) : y in u && (b = i(() => /* @__PURE__ */ A(he)), t.set(y + "", b));
          }
        if (m === void 0)
          (!_ || (Y = un(u, o)) != null && Y.writable) && (m = i(() => /* @__PURE__ */ A(void 0)), c(m, Be(v)), t.set(o, m));
        else {
          _ = m.v !== he;
          var z = i(() => Be(v));
          c(m, z);
        }
        var w = Reflect.getOwnPropertyDescriptor(u, o);
        if (w != null && w.set && w.set.call(d, v), !_) {
          if (n && typeof o == "string") {
            var F = (
              /** @type {Source<number>} */
              t.get("length")
            ), W = Number(o);
            Number.isInteger(W) && W >= F.v && c(F, W + 1);
          }
          vn(a);
        }
        return !0;
      },
      ownKeys(u) {
        s(a);
        var o = Reflect.ownKeys(u).filter((m) => {
          var _ = t.get(m);
          return _ === void 0 || _.v !== he;
        });
        for (var [v, d] of t)
          d.v !== he && !(v in u) && o.push(v);
        return o;
      },
      setPrototypeOf() {
        Al();
      }
    }
  );
}
var Jl, $l, ei;
function Wt(e = "") {
  return document.createTextNode(e);
}
// @__NO_SIDE_EFFECTS__
function je(e) {
  return (
    /** @type {TemplateNode | null} */
    $l.call(e)
  );
}
// @__NO_SIDE_EFFECTS__
function Ar(e) {
  return (
    /** @type {TemplateNode | null} */
    ei.call(e)
  );
}
function h(e, r) {
  return /* @__PURE__ */ je(e);
}
function dn(e, r = !1) {
  {
    var t = /* @__PURE__ */ je(e);
    return t instanceof Comment && t.data === "" ? /* @__PURE__ */ Ar(t) : t;
  }
}
function M(e, r = 1, t = !1) {
  let n = e;
  for (; r--; )
    n = /** @type {TemplateNode} */
    /* @__PURE__ */ Ar(n);
  return n;
}
function ti(e) {
  e.textContent = "";
}
function Pa() {
  return !1;
}
function Oa(e, r, t) {
  return (
    /** @type {T extends keyof HTMLElementTagNameMap ? HTMLElementTagNameMap[T] : Element} */
    document.createElementNS(r ?? va, e, void 0)
  );
}
let $n = !1;
function ri() {
  $n || ($n = !0, document.addEventListener(
    "reset",
    (e) => {
      Promise.resolve().then(() => {
        var r;
        if (!e.defaultPrevented)
          for (
            const t of
            /**@type {HTMLFormElement} */
            e.target.elements
          )
            (r = t.__on_r) == null || r.call(t);
      });
    },
    // In the capture phase to guarantee we get noticed of it (no possibility of stopPropagation)
    { capture: !0 }
  ));
}
function Xr(e) {
  var r = O, t = V;
  ht(null), Nt(null);
  try {
    return e();
  } finally {
    ht(r), Nt(t);
  }
}
function ni(e, r, t, n = t) {
  e.addEventListener(r, () => Xr(t));
  const a = e.__on_r;
  a ? e.__on_r = () => {
    a(), n(!0);
  } : e.__on_r = () => n(!0), ri();
}
function ai(e) {
  V === null && (O === null && Ml(), El()), Qt && kl();
}
function si(e, r) {
  var t = r.last;
  t === null ? r.last = r.first = e : (t.next = e, e.prev = t, r.last = e);
}
function Rt(e, r) {
  var t = V;
  t !== null && (t.f & Ae) !== 0 && (e |= Ae);
  var n = {
    ctx: lt,
    deps: null,
    nodes: null,
    f: e | ge | Qe,
    first: null,
    fn: r,
    last: null,
    next: null,
    parent: t,
    b: t && t.b,
    prev: null,
    teardown: null,
    wv: 0,
    ac: null
  };
  D == null || D.register_created_effect(n);
  var a = n;
  if ((e & ir) !== 0)
    tr !== null ? tr.push(n) : br.ensure().schedule(n);
  else if (r !== null) {
    try {
      fr(n);
    } catch (i) {
      throw xt(n), i;
    }
    a.deps === null && a.teardown === null && a.nodes === null && a.first === a.last && // either `null`, or a singular child
    (a.f & Yr) === 0 && (a = a.first, (e & kt) !== 0 && (e & mr) !== 0 && a !== null && (a.f |= mr));
  }
  if (a !== null && (a.parent = t, t !== null && si(a, t), O !== null && (O.f & ue) !== 0 && (e & or) === 0)) {
    var l = (
      /** @type {Derived} */
      O
    );
    (l.effects ?? (l.effects = [])).push(a);
  }
  return n;
}
function Fa() {
  return O !== null && !at;
}
function Pn(e) {
  const r = Rt(Gr, null);
  return te(r, le), r.teardown = e, r;
}
function li(e) {
  ai();
  var r = (
    /** @type {Effect} */
    V.f
  ), t = !O && (r & st) !== 0 && (r & Xt) === 0;
  if (t) {
    var n = (
      /** @type {ComponentContext} */
      lt
    );
    (n.e ?? (n.e = [])).push(e);
  } else
    return Ba(e);
}
function Ba(e) {
  return Rt(ir | bl, e);
}
function ja(e) {
  return Rt(ir, e);
}
function ii(e) {
  return Rt(Qr | Yr, e);
}
function On(e, r = 0) {
  return Rt(Gr | r, e);
}
function L(e, r = [], t = [], n = []) {
  Vl(n, r, t, (a) => {
    Rt(Gr, () => e(...a.map(s)));
  });
}
function Ha(e, r = 0) {
  var t = Rt(kt | r, e);
  return t;
}
function xr(e) {
  return Rt(st | Yr, e);
}
function za(e) {
  var r = e.teardown;
  if (r !== null) {
    const t = Qt, n = O;
    ea(!0), ht(null);
    try {
      r.call(null);
    } finally {
      ea(t), ht(n);
    }
  }
}
function Fn(e, r = !1) {
  var t = e.first;
  for (e.first = e.last = null; t !== null; ) {
    const a = t.ac;
    a !== null && Xr(() => {
      a.abort(bt);
    });
    var n = t.next;
    (t.f & or) !== 0 ? t.parent = null : xt(t, r), t = n;
  }
}
function oi(e) {
  for (var r = e.first; r !== null; ) {
    var t = r.next;
    (r.f & st) === 0 && xt(r), r = t;
  }
}
function xt(e, r = !0) {
  var t = !1;
  (r || (e.f & ml) !== 0) && e.nodes !== null && e.nodes.end !== null && (Ua(
    e.nodes.start,
    /** @type {TemplateNode} */
    e.nodes.end
  ), t = !0), te(e, mn), Fn(e, r && !t), kr(e, 0);
  var n = e.nodes && e.nodes.t;
  if (n !== null)
    for (const l of n)
      l.stop();
  za(e), e.f ^= mn, e.f |= Xe;
  var a = e.parent;
  a !== null && a.first !== null && Va(e), e.next = e.prev = e.teardown = e.ctx = e.deps = e.fn = e.nodes = e.ac = e.b = null;
}
function Ua(e, r) {
  for (; e !== null; ) {
    var t = e === r ? null : /* @__PURE__ */ Ar(e);
    e.remove(), e = t;
  }
}
function Va(e) {
  var r = e.parent, t = e.prev, n = e.next;
  t !== null && (t.next = n), n !== null && (n.prev = t), r !== null && (r.first === e && (r.first = n), r.last === e && (r.last = t));
}
function Bn(e, r, t = !0) {
  var n = [];
  Ka(e, n, !0);
  var a = () => {
    t && xt(e), r && r();
  }, l = n.length;
  if (l > 0) {
    var i = () => --l || a();
    for (var u of n)
      u.out(i);
  } else
    a();
}
function Ka(e, r, t) {
  if ((e.f & Ae) === 0) {
    e.f ^= Ae;
    var n = e.nodes && e.nodes.t;
    if (n !== null)
      for (const u of n)
        (u.is_global || t) && r.push(u);
    for (var a = e.first; a !== null; ) {
      var l = a.next, i = (a.f & mr) !== 0 || // If this is a branch effect without a block effect parent,
      // it means the parent block effect was pruned. In that case,
      // transparency information was transferred to the branch effect.
      (a.f & st) !== 0 && (e.f & kt) !== 0;
      Ka(a, r, i ? t : !1), a = l;
    }
  }
}
function jn(e) {
  Wa(e, !0);
}
function Wa(e, r) {
  if ((e.f & Ae) !== 0) {
    e.f ^= Ae, (e.f & le) === 0 && (te(e, ge), br.ensure().schedule(e));
    for (var t = e.first; t !== null; ) {
      var n = t.next, a = (t.f & mr) !== 0 || (t.f & st) !== 0;
      Wa(t, a ? r : !1), t = n;
    }
    var l = e.nodes && e.nodes.t;
    if (l !== null)
      for (const i of l)
        (i.is_global || r) && i.in();
  }
}
function Ga(e, r) {
  if (e.nodes)
    for (var t = e.nodes.start, n = e.nodes.end; t !== null; ) {
      var a = t === n ? null : /* @__PURE__ */ Ar(t);
      r.append(t), t = a;
    }
}
let Br = !1, Qt = !1;
function ea(e) {
  Qt = e;
}
let O = null, at = !1;
function ht(e) {
  O = e;
}
let V = null;
function Nt(e) {
  V = e;
}
let Ze = null;
function Ya(e) {
  O !== null && (Ze === null ? Ze = [e] : Ze.push(e));
}
let Se = null, Pe = 0, Ye = null;
function fi(e) {
  Ye = e;
}
let Qa = 1, zt = 0, Gt = zt;
function ta(e) {
  Gt = e;
}
function Xa() {
  return ++Qa;
}
function Cr(e) {
  var r = e.f;
  if ((r & ge) !== 0)
    return !0;
  if (r & ue && (e.f &= ~Yt), (r & _t) !== 0) {
    for (var t = (
      /** @type {Value[]} */
      e.deps
    ), n = t.length, a = 0; a < n; a++) {
      var l = t[a];
      if (Cr(
        /** @type {Derived} */
        l
      ) && Ia(
        /** @type {Derived} */
        l
      ), l.wv > e.wv)
        return !0;
    }
    (r & Qe) !== 0 && // During time traveling we don't want to reset the status so that
    // traversal of the graph in the other batches still happens
    pe === null && te(e, le);
  }
  return !1;
}
function Za(e, r, t = !0) {
  var n = e.reactions;
  if (n !== null && !(Ze !== null && lr.call(Ze, e)))
    for (var a = 0; a < n.length; a++) {
      var l = n[a];
      (l.f & ue) !== 0 ? Za(
        /** @type {Derived} */
        l,
        r,
        !1
      ) : r === l && (t ? te(l, ge) : (l.f & le) !== 0 && te(l, _t), Nn(
        /** @type {Effect} */
        l
      ));
    }
}
function Ja(e) {
  var z;
  var r = Se, t = Pe, n = Ye, a = O, l = Ze, i = lt, u = at, o = Gt, v = e.f;
  Se = /** @type {null | Value[]} */
  null, Pe = 0, Ye = null, O = (v & (st | or)) === 0 ? e : null, Ze = null, Hr(e.ctx), at = !1, Gt = ++zt, e.ac !== null && (Xr(() => {
    e.ac.abort(bt);
  }), e.ac = null);
  try {
    e.f |= yn;
    var d = (
      /** @type {Function} */
      e.fn
    ), m = d();
    e.f |= Xt;
    var _ = e.deps, y = D == null ? void 0 : D.is_fork;
    if (Se !== null) {
      var b;
      if (y || kr(e, Pe), _ !== null && Pe > 0)
        for (_.length = Pe + Se.length, b = 0; b < Se.length; b++)
          _[Pe + b] = Se[b];
      else
        e.deps = _ = Se;
      if (Fa() && (e.f & Qe) !== 0)
        for (b = Pe; b < _.length; b++)
          ((z = _[b]).reactions ?? (z.reactions = [])).push(e);
    } else !y && _ !== null && Pe < _.length && (kr(e, Pe), _.length = Pe);
    if (ya() && Ye !== null && !at && _ !== null && (e.f & (ue | _t | ge)) === 0)
      for (b = 0; b < /** @type {Source[]} */
      Ye.length; b++)
        Za(
          Ye[b],
          /** @type {Effect} */
          e
        );
    if (a !== null && a !== e) {
      if (zt++, a.deps !== null)
        for (let w = 0; w < t; w += 1)
          a.deps[w].rv = zt;
      if (r !== null)
        for (const w of r)
          w.rv = zt;
      Ye !== null && (n === null ? n = Ye : n.push(.../** @type {Source[]} */
      Ye));
    }
    return (e.f & It) !== 0 && (e.f ^= It), m;
  } catch (w) {
    return Rl(w);
  } finally {
    e.f ^= yn, Se = r, Pe = t, Ye = n, O = a, Ze = l, Hr(i), at = u, Gt = o;
  }
}
function ui(e, r) {
  let t = r.reactions;
  if (t !== null) {
    var n = _l.call(t, e);
    if (n !== -1) {
      var a = t.length - 1;
      a === 0 ? t = r.reactions = null : (t[n] = t[a], t.pop());
    }
  }
  if (t === null && (r.f & ue) !== 0 && // Destroying a child effect while updating a parent effect can cause a dependency to appear
  // to be unused, when in fact it is used by the currently-updating parent. Checking `new_deps`
  // allows us to skip the expensive work of disconnecting and immediately reconnecting it
  (Se === null || !lr.call(Se, r))) {
    var l = (
      /** @type {Derived} */
      r
    );
    (l.f & Qe) !== 0 && (l.f ^= Qe, l.f &= ~Yt), In(l), Xl(l), kr(l, 0);
  }
}
function kr(e, r) {
  var t = e.deps;
  if (t !== null)
    for (var n = r; n < t.length; n++)
      ui(e, t[n]);
}
function fr(e) {
  var r = e.f;
  if ((r & Xe) === 0) {
    te(e, le);
    var t = V, n = Br;
    V = e, Br = !0;
    try {
      (r & (kt | ga)) !== 0 ? oi(e) : Fn(e), za(e);
      var a = Ja(e);
      e.teardown = typeof a == "function" ? a : null, e.wv = Qa;
      var l;
      dl && Il && (e.f & ge) !== 0 && e.deps;
    } finally {
      Br = n, V = t;
    }
  }
}
async function ci() {
  await Promise.resolve(), zl();
}
function s(e) {
  var r = e.f, t = (r & ue) !== 0;
  if (O !== null && !at) {
    var n = V !== null && (V.f & Xe) !== 0;
    if (!n && (Ze === null || !lr.call(Ze, e))) {
      var a = O.deps;
      if ((O.f & yn) !== 0)
        e.rv < zt && (e.rv = zt, Se === null && a !== null && a[Pe] === e ? Pe++ : Se === null ? Se = [e] : Se.push(e));
      else {
        (O.deps ?? (O.deps = [])).push(e);
        var l = e.reactions;
        l === null ? e.reactions = [O] : lr.call(l, O) || l.push(O);
      }
    }
  }
  if (Qt && Dt.has(e))
    return Dt.get(e);
  if (t) {
    var i = (
      /** @type {Derived} */
      e
    );
    if (Qt) {
      var u = i.v;
      return ((i.f & le) === 0 && i.reactions !== null || es(i)) && (u = Rn(i)), Dt.set(i, u), u;
    }
    var o = (i.f & Qe) === 0 && !at && O !== null && (Br || (O.f & Qe) !== 0), v = (i.f & Xt) === 0;
    Cr(i) && (o && (i.f |= Qe), Ia(i)), o && !v && (Da(i), $a(i));
  }
  if (pe != null && pe.has(e))
    return pe.get(e);
  if ((e.f & It) !== 0)
    throw e.v;
  return e.v;
}
function $a(e) {
  if (e.f |= Qe, e.deps !== null)
    for (const r of e.deps)
      (r.reactions ?? (r.reactions = [])).push(e), (r.f & ue) !== 0 && (r.f & Qe) === 0 && (Da(
        /** @type {Derived} */
        r
      ), $a(
        /** @type {Derived} */
        r
      ));
}
function es(e) {
  if (e.v === he) return !0;
  if (e.deps === null) return !1;
  for (const r of e.deps)
    if (Dt.has(r) || (r.f & ue) !== 0 && es(
      /** @type {Derived} */
      r
    ))
      return !0;
  return !1;
}
function Zr(e) {
  var r = at;
  try {
    return at = !0, e();
  } finally {
    at = r;
  }
}
function vi(e) {
  if (!(typeof e != "object" || !e || e instanceof EventTarget)) {
    if (rr in e)
      Tn(e);
    else if (!Array.isArray(e))
      for (let r in e) {
        const t = e[r];
        typeof t == "object" && t && rr in t && Tn(t);
      }
  }
}
function Tn(e, r = /* @__PURE__ */ new Set()) {
  if (typeof e == "object" && e !== null && // We don't want to traverse DOM elements
  !(e instanceof EventTarget) && !r.has(e)) {
    r.add(e), e instanceof Date && e.getTime();
    for (let n in e)
      try {
        Tn(e[n], r);
      } catch {
      }
    const t = Cn(e);
    if (t !== Object.prototype && t !== Array.prototype && t !== Map.prototype && t !== Set.prototype && t !== Date.prototype) {
      const n = ha(t);
      for (let a in n) {
        const l = n[a].get;
        if (l)
          try {
            l.call(e);
          } catch {
          }
      }
    }
  }
}
const Ut = Symbol("events"), di = /* @__PURE__ */ new Set(), _i = /* @__PURE__ */ new Set();
function hi(e, r, t, n = {}) {
  function a(l) {
    if (n.capture || gi.call(r, l), !l.cancelBubble)
      return Xr(() => t == null ? void 0 : t.call(this, l));
  }
  return e.startsWith("pointer") || e.startsWith("touch") || e === "wheel" ? zr(() => {
    r.addEventListener(e, a, n);
  }) : r.addEventListener(e, a, n), a;
}
function _n(e, r, t, n, a) {
  var l = { capture: n, passive: a }, i = hi(e, r, t, l);
  (r === document.body || // @ts-ignore
  r === window || // @ts-ignore
  r === document || // Firefox has quirky behavior, it can happen that we still get "canplay" events when the element is already removed
  r instanceof HTMLMediaElement) && Pn(() => {
    r.removeEventListener(e, i, l);
  });
}
function be(e, r, t) {
  (r[Ut] ?? (r[Ut] = {}))[e] = t;
}
function pi(e) {
  for (var r = 0; r < e.length; r++)
    di.add(e[r]);
  for (var t of _i)
    t(e);
}
let ra = null;
function gi(e) {
  var w, F;
  var r = this, t = (
    /** @type {Node} */
    r.ownerDocument
  ), n = e.type, a = ((w = e.composedPath) == null ? void 0 : w.call(e)) || [], l = (
    /** @type {null | Element} */
    a[0] || e.target
  );
  ra = e;
  var i = 0, u = ra === e && e[Ut];
  if (u) {
    var o = a.indexOf(u);
    if (o !== -1 && (r === document || r === /** @type {any} */
    window)) {
      e[Ut] = r;
      return;
    }
    var v = a.indexOf(r);
    if (v === -1)
      return;
    o <= v && (i = o);
  }
  if (l = /** @type {Element} */
  a[i] || e.target, l !== r) {
    _a(e, "currentTarget", {
      configurable: !0,
      get() {
        return l || t;
      }
    });
    var d = O, m = V;
    ht(null), Nt(null);
    try {
      for (var _, y = []; l !== null; ) {
        var b = l.assignedSlot || l.parentNode || /** @type {any} */
        l.host || null;
        try {
          var z = (F = l[Ut]) == null ? void 0 : F[n];
          z != null && (!/** @type {any} */
          l.disabled || // DOM could've been updated already by the time this is reached, so we check this as well
          // -> the target could not have been disabled because it emits the event in the first place
          e.target === l) && z.call(l, e);
        } catch (W) {
          _ ? y.push(W) : _ = W;
        }
        if (e.cancelBubble || b === r || b === null)
          break;
        l = b;
      }
      if (_) {
        for (let W of y)
          queueMicrotask(() => {
            throw W;
          });
        throw _;
      }
    } finally {
      e[Ut] = r, delete e.currentTarget, ht(d), Nt(m);
    }
  }
}
var fa;
const hn = (
  // We gotta write it like this because after downleveling the pure comment may end up in the wrong location
  ((fa = globalThis == null ? void 0 : globalThis.window) == null ? void 0 : fa.trustedTypes) && /* @__PURE__ */ globalThis.window.trustedTypes.createPolicy("svelte-trusted-html", {
    /** @param {string} html */
    createHTML: (e) => e
  })
);
function wi(e) {
  return (
    /** @type {string} */
    (hn == null ? void 0 : hn.createHTML(e)) ?? e
  );
}
function ts(e) {
  var r = Oa("template");
  return r.innerHTML = wi(e.replaceAll("<!>", "<!---->")), r.content;
}
function Lt(e, r) {
  var t = (
    /** @type {Effect} */
    V
  );
  t.nodes === null && (t.nodes = { start: e, end: r, a: null, t: null });
}
// @__NO_SIDE_EFFECTS__
function P(e, r) {
  var t = (r & ca) !== 0, n = (r & ul) !== 0, a, l = !e.startsWith("<!>");
  return () => {
    a === void 0 && (a = ts(l ? e : "<!>" + e), t || (a = /** @type {TemplateNode} */
    /* @__PURE__ */ je(a)));
    var i = (
      /** @type {TemplateNode} */
      n || Jl ? document.importNode(a, !0) : a.cloneNode(!0)
    );
    if (t) {
      var u = (
        /** @type {TemplateNode} */
        /* @__PURE__ */ je(i)
      ), o = (
        /** @type {TemplateNode} */
        i.lastChild
      );
      Lt(u, o);
    } else
      Lt(i, i);
    return i;
  };
}
// @__NO_SIDE_EFFECTS__
function mi(e, r, t = "svg") {
  var n = !e.startsWith("<!>"), a = (r & ca) !== 0, l = `<${t}>${n ? e : "<!>" + e}</${t}>`, i;
  return () => {
    if (!i) {
      var u = (
        /** @type {DocumentFragment} */
        ts(l)
      ), o = (
        /** @type {Element} */
        /* @__PURE__ */ je(u)
      );
      if (a)
        for (i = document.createDocumentFragment(); /* @__PURE__ */ je(o); )
          i.appendChild(
            /** @type {TemplateNode} */
            /* @__PURE__ */ je(o)
          );
      else
        i = /** @type {Element} */
        /* @__PURE__ */ je(o);
    }
    var v = (
      /** @type {TemplateNode} */
      i.cloneNode(!0)
    );
    if (a) {
      var d = (
        /** @type {TemplateNode} */
        /* @__PURE__ */ je(v)
      ), m = (
        /** @type {TemplateNode} */
        v.lastChild
      );
      Lt(d, m);
    } else
      Lt(v, v);
    return v;
  };
}
// @__NO_SIDE_EFFECTS__
function Ir(e, r) {
  return /* @__PURE__ */ mi(e, r, "svg");
}
function mt(e = "") {
  {
    var r = Wt(e + "");
    return Lt(r, r), r;
  }
}
function bi() {
  var e = document.createDocumentFragment(), r = document.createComment(""), t = Wt();
  return e.append(r, t), Lt(r, t), e;
}
function E(e, r) {
  e !== null && e.before(
    /** @type {Node} */
    r
  );
}
function k(e, r) {
  var t = r == null ? "" : typeof r == "object" ? `${r}` : r;
  t !== (e.__t ?? (e.__t = e.nodeValue)) && (e.__t = t, e.nodeValue = `${t}`);
}
var nt, vt, Fe, Kt, Tr, Sr, Wr;
class yi {
  /**
   * @param {TemplateNode} anchor
   * @param {boolean} transition
   */
  constructor(r, t = !0) {
    /** @type {TemplateNode} */
    Tt(this, "anchor");
    /** @type {Map<Batch, Key>} */
    ee(this, nt, /* @__PURE__ */ new Map());
    /**
     * Map of keys to effects that are currently rendered in the DOM.
     * These effects are visible and actively part of the document tree.
     * Example:
     * ```
     * {#if condition}
     * 	foo
     * {:else}
     * 	bar
     * {/if}
     * ```
     * Can result in the entries `true->Effect` and `false->Effect`
     * @type {Map<Key, Effect>}
     */
    ee(this, vt, /* @__PURE__ */ new Map());
    /**
     * Similar to #onscreen with respect to the keys, but contains branches that are not yet
     * in the DOM, because their insertion is deferred.
     * @type {Map<Key, Branch>}
     */
    ee(this, Fe, /* @__PURE__ */ new Map());
    /**
     * Keys of effects that are currently outroing
     * @type {Set<Key>}
     */
    ee(this, Kt, /* @__PURE__ */ new Set());
    /**
     * Whether to pause (i.e. outro) on change, or destroy immediately.
     * This is necessary for `<svelte:element>`
     */
    ee(this, Tr, !0);
    /**
     * @param {Batch} batch
     */
    ee(this, Sr, (r) => {
      if (p(this, nt).has(r)) {
        var t = (
          /** @type {Key} */
          p(this, nt).get(r)
        ), n = p(this, vt).get(t);
        if (n)
          jn(n), p(this, Kt).delete(t);
        else {
          var a = p(this, Fe).get(t);
          a && (p(this, vt).set(t, a.effect), p(this, Fe).delete(t), a.fragment.lastChild.remove(), this.anchor.before(a.fragment), n = a.effect);
        }
        for (const [l, i] of p(this, nt)) {
          if (p(this, nt).delete(l), l === r)
            break;
          const u = p(this, Fe).get(i);
          u && (xt(u.effect), p(this, Fe).delete(i));
        }
        for (const [l, i] of p(this, vt)) {
          if (l === t || p(this, Kt).has(l)) continue;
          const u = () => {
            if (Array.from(p(this, nt).values()).includes(l)) {
              var v = document.createDocumentFragment();
              Ga(i, v), v.append(Wt()), p(this, Fe).set(l, { effect: i, fragment: v });
            } else
              xt(i);
            p(this, Kt).delete(l), p(this, vt).delete(l);
          };
          p(this, Tr) || !n ? (p(this, Kt).add(l), Bn(i, u, !1)) : u();
        }
      }
    });
    /**
     * @param {Batch} batch
     */
    ee(this, Wr, (r) => {
      p(this, nt).delete(r);
      const t = Array.from(p(this, nt).values());
      for (const [n, a] of p(this, Fe))
        t.includes(n) || (xt(a.effect), p(this, Fe).delete(n));
    });
    this.anchor = r, Ft(this, Tr, t);
  }
  /**
   *
   * @param {any} key
   * @param {null | ((target: TemplateNode) => void)} fn
   */
  ensure(r, t) {
    var n = (
      /** @type {Batch} */
      D
    ), a = Pa();
    if (t && !p(this, vt).has(r) && !p(this, Fe).has(r))
      if (a) {
        var l = document.createDocumentFragment(), i = Wt();
        l.append(i), p(this, Fe).set(r, {
          effect: xr(() => t(i)),
          fragment: l
        });
      } else
        p(this, vt).set(
          r,
          xr(() => t(this.anchor))
        );
    if (p(this, nt).set(n, r), a) {
      for (const [u, o] of p(this, vt))
        u === r ? n.unskip_effect(o) : n.skip_effect(o);
      for (const [u, o] of p(this, Fe))
        u === r ? n.unskip_effect(o.effect) : n.skip_effect(o.effect);
      n.oncommit(p(this, Sr)), n.ondiscard(p(this, Wr));
    } else
      p(this, Sr).call(this, n);
  }
}
nt = new WeakMap(), vt = new WeakMap(), Fe = new WeakMap(), Kt = new WeakMap(), Tr = new WeakMap(), Sr = new WeakMap(), Wr = new WeakMap();
function fe(e, r, t = !1) {
  var n = new yi(e), a = t ? mr : 0;
  function l(i, u) {
    n.ensure(i, u);
  }
  Ha(() => {
    var i = !1;
    r((u, o = 0) => {
      i = !0, l(o, u);
    }), i || l(-1, null);
  }, a);
}
function qi(e, r) {
  return r;
}
function xi(e, r, t) {
  for (var n = [], a = r.length, l, i = r.length, u = 0; u < a; u++) {
    let m = r[u];
    Bn(
      m,
      () => {
        if (l) {
          if (l.pending.delete(m), l.done.add(m), l.pending.size === 0) {
            var _ = (
              /** @type {Set<EachOutroGroup>} */
              e.outrogroups
            );
            Sn(e, An(l.done)), _.delete(l), _.size === 0 && (e.outrogroups = null);
          }
        } else
          i -= 1;
      },
      !1
    );
  }
  if (i === 0) {
    var o = n.length === 0 && t !== null;
    if (o) {
      var v = (
        /** @type {Element} */
        t
      ), d = (
        /** @type {Element} */
        v.parentNode
      );
      ti(d), d.append(v), e.items.clear();
    }
    Sn(e, r, !o);
  } else
    l = {
      pending: new Set(r),
      done: /* @__PURE__ */ new Set()
    }, (e.outrogroups ?? (e.outrogroups = /* @__PURE__ */ new Set())).add(l);
}
function Sn(e, r, t = !0) {
  var n;
  if (e.pending.size > 0) {
    n = /* @__PURE__ */ new Set();
    for (const i of e.pending.values())
      for (const u of i)
        n.add(
          /** @type {EachItem} */
          e.items.get(u).e
        );
  }
  for (var a = 0; a < r.length; a++) {
    var l = r[a];
    if (n != null && n.has(l)) {
      l.f |= dt;
      const i = document.createDocumentFragment();
      Ga(l, i);
    } else
      xt(r[a], t);
  }
}
var na;
function pn(e, r, t, n, a, l = null) {
  var i = e, u = /* @__PURE__ */ new Map(), o = (r & ua) !== 0;
  if (o) {
    var v = (
      /** @type {Element} */
      e
    );
    i = v.appendChild(Wt());
  }
  var d = null, m = /* @__PURE__ */ Gl(() => {
    var Y = t();
    return da(Y) ? Y : Y == null ? [] : An(Y);
  }), _, y = /* @__PURE__ */ new Map(), b = !0;
  function z(Y) {
    (W.effect.f & Xe) === 0 && (W.pending.delete(Y), W.fallback = d, ki(W, _, i, r, n), d !== null && (_.length === 0 ? (d.f & dt) === 0 ? jn(d) : (d.f ^= dt, gr(d, null, i)) : Bn(d, () => {
      d = null;
    })));
  }
  function w(Y) {
    W.pending.delete(Y);
  }
  var F = Ha(() => {
    _ = /** @type {V[]} */
    s(m);
    for (var Y = _.length, ie = /* @__PURE__ */ new Set(), ye = (
      /** @type {Batch} */
      D
    ), H = Pa(), K = 0; K < Y; K += 1) {
      var Ce = _[K], qe = n(Ce, K), Z = b ? null : u.get(qe);
      Z ? (Z.v && qr(Z.v, Ce), Z.i && qr(Z.i, K), H && ye.unskip_effect(Z.e)) : (Z = Ei(
        u,
        b ? i : na ?? (na = Wt()),
        Ce,
        qe,
        K,
        a,
        r,
        t
      ), b || (Z.e.f |= dt), u.set(qe, Z)), ie.add(qe);
    }
    if (Y === 0 && l && !d && (b ? d = xr(() => l(i)) : (d = xr(() => l(na ?? (na = Wt()))), d.f |= dt)), Y > ie.size && xl(), !b)
      if (y.set(ye, ie), H) {
        for (const [it, Ie] of u)
          ie.has(it) || ye.skip_effect(Ie.e);
        ye.oncommit(z), ye.ondiscard(w);
      } else
        z(ye);
    s(m);
  }), W = { effect: F, items: u, pending: y, outrogroups: null, fallback: d };
  b = !1;
}
function hr(e) {
  for (; e !== null && (e.f & st) === 0; )
    e = e.next;
  return e;
}
function ki(e, r, t, n, a) {
  var Z, it, Ie, pt, Et, Zt, Pt, xe, De;
  var l = (n & ol) !== 0, i = r.length, u = e.items, o = hr(e.effect.first), v, d = null, m, _ = [], y = [], b, z, w, F;
  if (l)
    for (F = 0; F < i; F += 1)
      b = r[F], z = a(b, F), w = /** @type {EachItem} */
      u.get(z).e, (w.f & dt) === 0 && ((it = (Z = w.nodes) == null ? void 0 : Z.a) == null || it.measure(), (m ?? (m = /* @__PURE__ */ new Set())).add(w));
  for (F = 0; F < i; F += 1) {
    if (b = r[F], z = a(b, F), w = /** @type {EachItem} */
    u.get(z).e, e.outrogroups !== null)
      for (const ce of e.outrogroups)
        ce.pending.delete(w), ce.done.delete(w);
    if ((w.f & Ae) !== 0 && (jn(w), l && ((pt = (Ie = w.nodes) == null ? void 0 : Ie.a) == null || pt.unfix(), (m ?? (m = /* @__PURE__ */ new Set())).delete(w))), (w.f & dt) !== 0)
      if (w.f ^= dt, w === o)
        gr(w, null, t);
      else {
        var W = d ? d.next : o;
        w === e.effect.last && (e.effect.last = w.prev), w.prev && (w.prev.next = w.next), w.next && (w.next.prev = w.prev), At(e, d, w), At(e, w, W), gr(w, W, t), d = w, _ = [], y = [], o = hr(d.next);
        continue;
      }
    if (w !== o) {
      if (v !== void 0 && v.has(w)) {
        if (_.length < y.length) {
          var Y = y[0], ie;
          d = Y.prev;
          var ye = _[0], H = _[_.length - 1];
          for (ie = 0; ie < _.length; ie += 1)
            gr(_[ie], Y, t);
          for (ie = 0; ie < y.length; ie += 1)
            v.delete(y[ie]);
          At(e, ye.prev, H.next), At(e, d, ye), At(e, H, Y), o = Y, d = H, F -= 1, _ = [], y = [];
        } else
          v.delete(w), gr(w, o, t), At(e, w.prev, w.next), At(e, w, d === null ? e.effect.first : d.next), At(e, d, w), d = w;
        continue;
      }
      for (_ = [], y = []; o !== null && o !== w; )
        (v ?? (v = /* @__PURE__ */ new Set())).add(o), y.push(o), o = hr(o.next);
      if (o === null)
        continue;
    }
    (w.f & dt) === 0 && _.push(w), d = w, o = hr(w.next);
  }
  if (e.outrogroups !== null) {
    for (const ce of e.outrogroups)
      ce.pending.size === 0 && (Sn(e, An(ce.done)), (Et = e.outrogroups) == null || Et.delete(ce));
    e.outrogroups.size === 0 && (e.outrogroups = null);
  }
  if (o !== null || v !== void 0) {
    var K = [];
    if (v !== void 0)
      for (w of v)
        (w.f & Ae) === 0 && K.push(w);
    for (; o !== null; )
      (o.f & Ae) === 0 && o !== e.fallback && K.push(o), o = hr(o.next);
    var Ce = K.length;
    if (Ce > 0) {
      var qe = (n & ua) !== 0 && i === 0 ? t : null;
      if (l) {
        for (F = 0; F < Ce; F += 1)
          (Pt = (Zt = K[F].nodes) == null ? void 0 : Zt.a) == null || Pt.measure();
        for (F = 0; F < Ce; F += 1)
          (De = (xe = K[F].nodes) == null ? void 0 : xe.a) == null || De.fix();
      }
      xi(e, K, qe);
    }
  }
  l && zr(() => {
    var ce, ke;
    if (m !== void 0)
      for (w of m)
        (ke = (ce = w.nodes) == null ? void 0 : ce.a) == null || ke.apply();
  });
}
function Ei(e, r, t, n, a, l, i, u) {
  var o = (i & ll) !== 0 ? (i & fl) === 0 ? /* @__PURE__ */ La(t, !1, !1) : yr(t) : null, v = (i & il) !== 0 ? yr(a) : null;
  return {
    v: o,
    i: v,
    e: xr(() => (l(r, o ?? t, v ?? a, u), () => {
      e.delete(n);
    }))
  };
}
function gr(e, r, t) {
  if (e.nodes)
    for (var n = e.nodes.start, a = e.nodes.end, l = r && (r.f & dt) === 0 ? (
      /** @type {EffectNodes} */
      r.nodes.start
    ) : t; n !== null; ) {
      var i = (
        /** @type {TemplateNode} */
        /* @__PURE__ */ Ar(n)
      );
      if (l.before(n), n === a)
        return;
      n = i;
    }
}
function At(e, r, t) {
  r === null ? e.effect.first = t : r.next = t, t === null ? e.effect.last = r : t.prev = r;
}
function Mi(e, r, t = !1, n = !1, a = !1, l = !1) {
  var i = e, u = "";
  if (t)
    var o = (
      /** @type {Element} */
      e
    );
  L(() => {
    var v = (
      /** @type {Effect} */
      V
    );
    if (u !== (u = r() ?? "")) {
      if (t) {
        v.nodes = null, o.innerHTML = /** @type {string} */
        u, u !== "" && Lt(
          /** @type {TemplateNode} */
          /* @__PURE__ */ je(o),
          /** @type {TemplateNode} */
          o.lastChild
        );
        return;
      }
      if (v.nodes !== null && (Ua(
        v.nodes.start,
        /** @type {TemplateNode} */
        v.nodes.end
      ), v.nodes = null), u !== "") {
        var d = n ? cl : a ? vl : void 0, m = (
          /** @type {HTMLTemplateElement | SVGElement | MathMLElement} */
          Oa(n ? "svg" : a ? "math" : "template", d)
        );
        m.innerHTML = /** @type {any} */
        u;
        var _ = n || a ? m : (
          /** @type {HTMLTemplateElement} */
          m.content
        );
        if (Lt(
          /** @type {TemplateNode} */
          /* @__PURE__ */ je(_),
          /** @type {TemplateNode} */
          _.lastChild
        ), n || a)
          for (; /* @__PURE__ */ je(_); )
            i.before(
              /** @type {TemplateNode} */
              /* @__PURE__ */ je(_)
            );
        else
          i.before(_);
      }
    }
  });
}
function aa(e, r, t) {
  ja(() => {
    var n = Zr(() => r(e, t == null ? void 0 : t()) || {});
    if (t && (n != null && n.update)) {
      var a = !1, l = (
        /** @type {any} */
        {}
      );
      On(() => {
        var i = t();
        vi(i), a && ma(l, i) && (l = i, n.update(i));
      }), a = !0;
    }
    if (n != null && n.destroy)
      return () => (
        /** @type {Function} */
        n.destroy()
      );
  });
}
const sa = [...` 	
\r\f \v\uFEFF`];
function Ti(e, r, t) {
  var n = e == null ? "" : "" + e;
  if (t) {
    for (var a of Object.keys(t))
      if (t[a])
        n = n ? n + " " + a : a;
      else if (n.length)
        for (var l = a.length, i = 0; (i = n.indexOf(a, i)) >= 0; ) {
          var u = i + l;
          (i === 0 || sa.includes(n[i - 1])) && (u === n.length || sa.includes(n[u])) ? n = (i === 0 ? "" : n.substring(0, i)) + n.substring(u + 1) : i = u;
        }
  }
  return n === "" ? null : n;
}
function Si(e, r) {
  return e == null ? null : String(e);
}
function Rr(e, r, t, n, a, l) {
  var i = e.__className;
  if (i !== t || i === void 0) {
    var u = Ti(t, n, l);
    u == null ? e.removeAttribute("class") : r ? e.className = u : e.setAttribute("class", u), e.__className = t;
  } else if (l && a !== l)
    for (var o in l) {
      var v = !!l[o];
      (a == null || v !== !!a[o]) && e.classList.toggle(o, v);
    }
  return l;
}
function Ai(e, r, t, n) {
  var a = e.__style;
  if (a !== r) {
    var l = Si(r);
    l == null ? e.removeAttribute("style") : e.style.cssText = l, e.__style = r;
  }
  return n;
}
const Ci = Symbol("is custom element"), Ii = Symbol("is html");
function jt(e, r, t, n) {
  var a = Di(e);
  a[r] !== (a[r] = t) && (r === "loading" && (e[yl] = t), t == null ? e.removeAttribute(r) : typeof t != "string" && Ni(e).includes(r) ? e[r] = t : e.setAttribute(r, t));
}
function Di(e) {
  return (
    /** @type {Record<string | symbol, unknown>} **/
    // @ts-expect-error
    e.__attributes ?? (e.__attributes = {
      [Ci]: e.nodeName.includes("-"),
      [Ii]: e.namespaceURI === va
    })
  );
}
var la = /* @__PURE__ */ new Map();
function Ni(e) {
  var r = e.getAttribute("is") || e.nodeName, t = la.get(r);
  if (t) return t;
  la.set(r, t = []);
  for (var n, a = e, l = Element.prototype; l !== a; ) {
    n = ha(a);
    for (var i in n)
      n[i].set && t.push(i);
    a = Cn(a);
  }
  return t;
}
function pr(e, r, t = r) {
  var n = /* @__PURE__ */ new WeakSet();
  ni(e, "input", async (a) => {
    var l = a ? e.defaultValue : e.value;
    if (l = gn(e) ? wn(l) : l, t(l), D !== null && n.add(D), await ci(), l !== (l = r())) {
      var i = e.selectionStart, u = e.selectionEnd, o = e.value.length;
      if (e.value = l ?? "", u !== null) {
        var v = e.value.length;
        i === u && u === o && v > o ? (e.selectionStart = v, e.selectionEnd = v) : (e.selectionStart = i, e.selectionEnd = Math.min(u, v));
      }
    }
  }), // If we are hydrating and the value has since changed,
  // then use the updated value from the input instead.
  // If defaultValue is set, then value == defaultValue
  // TODO Svelte 6: remove input.value check and set to empty string?
  Zr(r) == null && e.value && (t(gn(e) ? wn(e.value) : e.value), D !== null && n.add(D)), On(() => {
    var a = r();
    if (e === document.activeElement) {
      var l = (
        /** @type {Batch} */
        D
      );
      if (n.has(l))
        return;
    }
    gn(e) && a === wn(e.value) || e.type === "date" && !a && !e.value || a !== e.value && (e.value = a ?? "");
  });
}
function gn(e) {
  var r = e.type;
  return r === "number" || r === "range";
}
function wn(e) {
  return e === "" ? null : +e;
}
function ia(e, r) {
  return e === r || (e == null ? void 0 : e[rr]) === r;
}
function Li(e = {}, r, t, n) {
  var a = (
    /** @type {ComponentContext} */
    lt.r
  ), l = (
    /** @type {Effect} */
    V
  );
  return ja(() => {
    var i, u;
    return On(() => {
      i = u, u = [], Zr(() => {
        e !== t(...u) && (r(e, ...u), i && ia(t(...i), e) && r(null, ...i));
      });
    }), () => {
      let o = l;
      for (; o !== a && o.parent !== null && o.parent.f & mn; )
        o = o.parent;
      const v = () => {
        u && ia(t(...u), e) && r(null, ...u);
      }, d = o.teardown;
      o.teardown = () => {
        v(), d == null || d();
      };
    };
  }), e;
}
var Ri = /* @__PURE__ */ P('<div class="page-center svelte-1wqcg45"><span class="spinner svelte-1wqcg45"></span> <span class="spinner-text svelte-1wqcg45"> </span></div>'), Pi = /* @__PURE__ */ P('<div class="qr-placeholder svelte-1wqcg45"><span class="spinner svelte-1wqcg45"></span> <span class="spinner-text svelte-1wqcg45"> </span></div>'), Oi = /* @__PURE__ */ P('<div class="qr-placeholder svelte-1wqcg45"><p class="error-msg svelte-1wqcg45"> </p> <button class="button"> </button></div>'), Fi = /* @__PURE__ */ P('<div class="qr-container svelte-1wqcg45"></div>'), Bi = /* @__PURE__ */ P('<div class="page-center svelte-1wqcg45"><div class="login-card svelte-1wqcg45"><h2 class="svelte-1wqcg45"> <!></h2> <!> <div class="qr-text svelte-1wqcg45"><h3 class="svelte-1wqcg45"> </h3> <p class="qr-instruction svelte-1wqcg45"> </p></div> <div class="separator svelte-1wqcg45"><span class="separator-line svelte-1wqcg45"></span> <span class="separator-text svelte-1wqcg45"> </span> <span class="separator-line svelte-1wqcg45"></span></div> <button class="button use-phone-btn svelte-1wqcg45"> </button></div></div>'), ji = /* @__PURE__ */ P('<p class="error-msg svelte-1wqcg45"> </p>'), Hi = /* @__PURE__ */ P('<div class="page-center svelte-1wqcg45"><div class="login-card svelte-1wqcg45"><h2 class="svelte-1wqcg45"> </h2> <form class="form svelte-1wqcg45"><label class="field svelte-1wqcg45"><span class="field-label svelte-1wqcg45"> </span> <input type="tel" class="input svelte-1wqcg45" required=""/> <span class="field-hint svelte-1wqcg45"> </span></label> <!> <button type="submit" class="button"> </button></form> <button class="button back-to-qr-btn svelte-1wqcg45"> </button></div></div>'), zi = /* @__PURE__ */ P('<p class="error-msg svelte-1wqcg45"> </p>'), Ui = /* @__PURE__ */ P('<div class="page-center svelte-1wqcg45"><div class="login-card svelte-1wqcg45"><h2 class="svelte-1wqcg45"> </h2> <form class="form svelte-1wqcg45"><label class="field svelte-1wqcg45"><span class="field-label svelte-1wqcg45"> </span> <input type="text" inputmode="numeric" class="input svelte-1wqcg45" required=""/> <span class="field-hint svelte-1wqcg45"> </span></label> <!> <button type="submit" class="button"> </button></form></div></div>'), Vi = /* @__PURE__ */ P('<span class="field-hint svelte-1wqcg45"> </span>'), Ki = /* @__PURE__ */ P('<p class="error-msg svelte-1wqcg45"> </p>'), Wi = /* @__PURE__ */ P('<div class="page-center svelte-1wqcg45"><div class="login-card svelte-1wqcg45"><h2 class="svelte-1wqcg45"> </h2> <form class="form svelte-1wqcg45"><label class="field svelte-1wqcg45"><span class="field-label svelte-1wqcg45"> </span> <input type="password" class="input svelte-1wqcg45" required=""/> <!></label> <!> <button type="submit" class="button"> </button></form></div></div>'), Gi = /* @__PURE__ */ P('<div class="spinner-section svelte-1wqcg45"><span class="spinner svelte-1wqcg45"></span> <span class="spinner-text svelte-1wqcg45"> </span></div>'), Yi = /* @__PURE__ */ P('<div class="error-section svelte-1wqcg45"><p class="error-msg svelte-1wqcg45"> </p> <button class="button"> </button></div>'), Qi = /* @__PURE__ */ P('<p class="empty-text svelte-1wqcg45"> </p>'), Xi = /* @__PURE__ */ P('<img alt="" class="chat-photo-img svelte-1wqcg45"/>'), Zi = /* @__PURE__ */ P('<button class="chat-item button svelte-1wqcg45"><div><!></div> <div class="chat-info svelte-1wqcg45"><span class="chat-title svelte-1wqcg45"> </span> <span class="chat-type svelte-1wqcg45"> </span></div> <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" class="chat-arrow svelte-1wqcg45"><path d="M9 6l6 6-6 6"></path></svg></button>'), Ji = /* @__PURE__ */ P('<div class="chats-header svelte-1wqcg45"><h2 class="svelte-1wqcg45"> </h2> <span class="subtext svelte-1wqcg45"> </span></div> <input type="text" class="input search-input svelte-1wqcg45" placeholder="Search..."/> <div class="chats-list svelte-1wqcg45"></div>', 1), $i = /* @__PURE__ */ P('<div class="page-logged svelte-1wqcg45"><div class="session-bar svelte-1wqcg45"><span class="session-info svelte-1wqcg45"> </span> <div class="session-actions svelte-1wqcg45"><button class="button"><svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 2v6h-6"></path><path d="M3 12a9 9 0 0115-6.7L21 8"></path><path d="M3 22v-6h6"></path><path d="M21 12a9 9 0 01-15 6.7L3 16"></path></svg></button> <button class="button"> </button></div></div> <!></div>'), eo = /* @__PURE__ */ P("<button> </button>"), to = /* @__PURE__ */ P('<div class="spinner-section svelte-1wqcg45"><span class="spinner svelte-1wqcg45"></span> <span class="spinner-text svelte-1wqcg45"> </span></div>'), ro = /* @__PURE__ */ P('<div class="error-section svelte-1wqcg45"><p class="error-msg svelte-1wqcg45"> </p> <button class="button"> </button></div>'), no = /* @__PURE__ */ P('<p class="empty-text svelte-1wqcg45"> </p>'), ao = /* @__PURE__ */ P('<button class="button batch-cancel-btn svelte-1wqcg45"> </button>'), so = /* @__PURE__ */ P('<button class="button batch-download-btn svelte-1wqcg45"> </button>'), lo = /* @__PURE__ */ P('<div class="batch-progress-section svelte-1wqcg45"><div class="batch-progress-bar-outer svelte-1wqcg45"><div class="batch-progress-bar-inner svelte-1wqcg45"></div></div> <span class="subtext svelte-1wqcg45"> </span></div>'), io = /* @__PURE__ */ P('<img alt="" class="thumb-img svelte-1wqcg45"/>'), oo = /* @__PURE__ */ Ir('<rect x="3" y="3" width="18" height="18" rx="2"></rect><circle cx="8.5" cy="8.5" r="1.5"></circle><path d="M21 15l-5-5L5 21"></path>', 1), fo = /* @__PURE__ */ Ir('<rect x="2" y="5" width="20" height="14" rx="2"></rect><path d="M10 9l5 3-5 3z" fill="currentColor" stroke="none"></path>', 1), uo = /* @__PURE__ */ Ir('<path d="M9 18V5l12-2v13"></path><circle cx="6" cy="18" r="3"></circle><circle cx="18" cy="16" r="3"></circle>', 1), co = /* @__PURE__ */ Ir('<path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"></path><path d="M14 2v6h6"></path>', 1), vo = /* @__PURE__ */ Ir('<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><!></svg>'), _o = /* @__PURE__ */ P('<span class="media-status-icon downloading svelte-1wqcg45"><span class="spinner small svelte-1wqcg45"></span></span>'), ho = /* @__PURE__ */ P('<span class="media-status-icon done svelte-1wqcg45"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="var(--green)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6L9 17l-5-5"></path></svg></span>'), po = /* @__PURE__ */ P('<span class="media-status-icon skipped svelte-1wqcg45"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="var(--gray)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M13 17l5-5-5-5"></path><path d="M6 17l5-5-5-5"></path></svg></span>'), go = /* @__PURE__ */ P('<span class="media-status-icon error-icon svelte-1wqcg45"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="var(--red)" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6L6 18"></path><path d="M6 6l12 12"></path></svg></span>'), wo = /* @__PURE__ */ P('<span class="media-status-icon waiting svelte-1wqcg45"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="var(--gray)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"></circle><path d="M12 6v6l4 2"></path></svg></span>'), mo = /* @__PURE__ */ P('<button class="button media-download-btn svelte-1wqcg45"><!></button>'), bo = /* @__PURE__ */ P('<div class="media-item svelte-1wqcg45"><div><!></div> <div class="media-info svelte-1wqcg45"><span class="media-name svelte-1wqcg45"> </span> <span class="media-meta svelte-1wqcg45"> <!></span></div> <!></div>'), yo = /* @__PURE__ */ P('<span class="spinner small svelte-1wqcg45"></span>'), qo = /* @__PURE__ */ P('<button class="button load-more-btn svelte-1wqcg45"><!></button>'), xo = /* @__PURE__ */ P('<div class="media-header svelte-1wqcg45"><span class="subtext svelte-1wqcg45"> </span> <div class="media-header-actions svelte-1wqcg45"><!></div></div> <!> <div class="media-list svelte-1wqcg45"></div> <!>', 1), ko = /* @__PURE__ */ P('<div class="page-logged svelte-1wqcg45"><div class="session-bar svelte-1wqcg45"><button class="button back-btn svelte-1wqcg45"><svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 18l-6-6 6-6"></path></svg> </button> <span class="session-info svelte-1wqcg45"> </span></div> <div class="filters svelte-1wqcg45"></div> <input type="text" class="input search-input svelte-1wqcg45"/> <!></div>');
function Ro(e, r) {
  Dl(r, !0);
  const t = () => Bl(al, "$t", n), [n, a] = jl();
  let l = /* @__PURE__ */ A("checking"), i = /* @__PURE__ */ A(""), u = /* @__PURE__ */ A(""), o = /* @__PURE__ */ A(""), v = /* @__PURE__ */ A(""), d = /* @__PURE__ */ A(""), m = /* @__PURE__ */ A(!1), _ = /* @__PURE__ */ A(""), y = /* @__PURE__ */ A(""), b = /* @__PURE__ */ A(!1), z = /* @__PURE__ */ A(""), w = /* @__PURE__ */ A(null), F = /* @__PURE__ */ A(null), W = /* @__PURE__ */ A(Be([])), Y = /* @__PURE__ */ A(!1), ie = /* @__PURE__ */ A(""), ye = /* @__PURE__ */ A(""), H = /* @__PURE__ */ A(null), K = /* @__PURE__ */ A(Be([])), Ce = /* @__PURE__ */ A(!1), qe = /* @__PURE__ */ A(""), Z = /* @__PURE__ */ A("all"), it = /* @__PURE__ */ A(!1), Ie = /* @__PURE__ */ A(!0), pt = /* @__PURE__ */ A(""), Et = null, Zt = /* @__PURE__ */ A(!1), Pt = /* @__PURE__ */ A(null), xe = /* @__PURE__ */ A(Be(/* @__PURE__ */ new Map())), De = /* @__PURE__ */ A(null), ce = /* @__PURE__ */ A(0), ke = /* @__PURE__ */ A(0), Jt = /* @__PURE__ */ ft(() => s(De) !== null), rs = /* @__PURE__ */ ft(() => s(ke) > 0 ? s(ce) / s(ke) * 100 : 0), Je = /* @__PURE__ */ A(Be(/* @__PURE__ */ new Set())), Mt = /* @__PURE__ */ A(Be(/* @__PURE__ */ new Map())), Ot = /* @__PURE__ */ A(Be(/* @__PURE__ */ new Map())), Jr = [], gt = /* @__PURE__ */ A(Be(/* @__PURE__ */ new Map())), He = /* @__PURE__ */ A(Be(/* @__PURE__ */ new Map())), Dr = 0, Nr = 0;
  const ns = 5, Lr = [];
  let as = /* @__PURE__ */ ft(() => s(ye).trim() ? s(W).filter((f) => f.title.toLowerCase().includes(s(ye).trim().toLowerCase())) : s(W));
  function Hn(f) {
    (f.ctrlKey || f.metaKey) && f.key === "f" && s(l) === "media" && s(Pt) && (f.preventDefault(), s(Pt).focus());
  }
  li(() => (zn(), ss(), Xn(ls), document.addEventListener("keydown", Hn), () => {
    ur(), Xn(null), $r(), document.removeEventListener("keydown", Hn), Et && clearTimeout(Et), _e("telegram", "telegram_clear_thumbnail_cache").catch(() => {
    });
    for (const f of Jr) f();
    Jr = [];
  }));
  async function ss() {
    const f = await Yn("generic-download-progress", (q) => {
      const S = q.payload;
      if (S.platform !== "telegram") return;
      const B = s(Mt).get(S.id);
      B !== void 0 && (s(Ot).set(B, S.percent), c(Ot, new Map(s(Ot)), !0));
    }), g = await Yn("generic-download-complete", (q) => {
      const S = q.payload;
      if (S.platform !== "telegram") return;
      const B = s(Mt).get(S.id);
      B !== void 0 && (c(Je, new Set([...s(Je)].filter((ae) => ae !== B)), !0), s(Mt).delete(S.id), c(Mt, new Map(s(Mt)), !0), s(Ot).delete(B), c(Ot, new Map(s(Ot)), !0), S.success ? Bt("success", t()("toast.download_complete", { name: S.title })) : Bt("error", S.error ?? t()("common.error")));
    });
    Jr = [f, g];
  }
  function ls(f) {
    if (f.batch_id !== s(De)) return;
    s(xe).set(f.message_id, { status: f.status, percent: f.percent }), c(xe, new Map(s(xe)), !0);
    let g = 0;
    for (const [, q] of s(xe))
      (q.status === "done" || q.status === "error" || q.status === "skipped") && g++;
    c(ce, g, !0), g >= s(ke) && s(ke) > 0 && c(De, null);
  }
  function ur() {
    s(w) && (clearInterval(s(w)), c(w, null)), s(F) && (clearTimeout(s(F)), c(F, null));
  }
  async function zn() {
    c(l, "checking");
    try {
      const f = await _e("telegram", "telegram_check_session");
      c(d, f, !0), c(l, "chats"), $t();
    } catch {
      c(l, "qr"), cr();
    }
  }
  async function cr() {
    c(b, !0), c(z, ""), c(y, ""), ur();
    try {
      const f = await _e("telegram", "telegram_qr_start");
      c(y, f.svg, !0), c(b, !1);
      const g = Math.floor(Date.now() / 1e3), q = Math.max((f.expires - g) * 1e3 - 2e3, 5e3);
      c(
        F,
        setTimeout(
          () => {
            s(l) === "qr" && cr();
          },
          q
        ),
        !0
      ), c(w, setInterval(is, 1500), !0);
    } catch (f) {
      c(b, !1);
      const g = typeof f == "string" ? f : f.message ?? "";
      g.includes("already_authenticated") ? zn() : c(z, g || t()("telegram.qr_error"), !0);
    }
  }
  async function is() {
    try {
      const f = await _e("telegram", "telegram_qr_poll");
      if (f === "waiting") return;
      ur(), f === "password_required" || f.startsWith("password_required:") ? (c(v, f.startsWith("password_required:") ? f.slice(18) : "", !0), c(l, "password")) : f.startsWith("success:") && (c(d, f.slice(8), !0), c(l, "chats"), $t());
    } catch {
    }
  }
  function os() {
    ur(), c(l, "phone");
  }
  function fs() {
    c(_, ""), c(l, "qr"), cr();
  }
  async function us() {
    c(_, ""), c(m, !0);
    try {
      await _e("telegram", "telegram_send_code", { phone: s(i).trim() }), c(l, "code");
    } catch (f) {
      c(_, typeof f == "string" ? f : f.message ?? t()("telegram.unknown_error"), !0);
    } finally {
      c(m, !1);
    }
  }
  async function cs() {
    c(_, ""), c(m, !0);
    try {
      const f = await _e("telegram", "telegram_verify_code", { code: s(u).trim() });
      c(d, f, !0), c(l, "chats"), $t();
    } catch (f) {
      const g = typeof f == "string" ? f : f.message ?? "";
      g === "invalid_code" ? c(_, t()("telegram.invalid_code"), !0) : g.startsWith("password_required:") ? (c(v, g.slice(18), !0), c(l, "password")) : c(_, g || t()("telegram.unknown_error"), !0);
    } finally {
      c(m, !1);
    }
  }
  async function vs() {
    c(_, ""), c(m, !0);
    try {
      const f = await _e("telegram", "telegram_verify_2fa", { password: s(o) });
      c(d, f, !0), c(l, "chats"), $t();
    } catch (f) {
      const g = typeof f == "string" ? f : f.message ?? "";
      g === "invalid_password" ? c(_, t()("telegram.invalid_password"), !0) : c(_, g || t()("telegram.unknown_error"), !0);
    } finally {
      c(m, !1);
    }
  }
  async function ds() {
    ur();
    try {
      await _e("telegram", "telegram_logout");
    } catch {
    }
    c(d, ""), c(W, [], !0), c(K, [], !0), c(H, null), c(gt, /* @__PURE__ */ new Map(), !0), c(i, ""), c(u, ""), c(o, ""), c(_, ""), c(l, "qr"), cr();
  }
  async function $t() {
    c(Y, !0), c(ie, "");
    try {
      c(W, await _e("telegram", "telegram_list_chats"), !0);
    } catch (f) {
      c(ie, typeof f == "string" ? f : f.message ?? t()("telegram.chats_error"), !0);
    } finally {
      c(Y, !1);
    }
  }
  async function _s(f) {
    c(H, f, !0), c(Z, "all"), c(pt, ""), c(l, "media"), c(xe, /* @__PURE__ */ new Map(), !0), c(De, null), c(ce, 0), c(ke, 0), c(Je, /* @__PURE__ */ new Set(), !0), $r(), c(Ie, !0), vr();
  }
  function hs() {
    c(H, null), c(K, [], !0), c(qe, ""), c(xe, /* @__PURE__ */ new Map(), !0), c(De, null), c(ce, 0), c(ke, 0), c(Je, /* @__PURE__ */ new Set(), !0), $r(), c(l, "chats");
  }
  async function vr() {
    if (s(H)) {
      c(Ce, !0), c(qe, "");
      try {
        const f = await _e("telegram", "telegram_list_media", {
          chatId: s(H).id,
          chatType: s(H).chat_type,
          mediaType: s(Z) === "all" ? null : s(Z),
          offset: 0,
          limit: 100
        });
        c(K, f, !0), c(Ie, f.length >= 100);
      } catch (f) {
        c(qe, typeof f == "string" ? f : f.message ?? t()("telegram.media_error"), !0);
      } finally {
        c(Ce, !1);
      }
    }
  }
  async function ps() {
    if (!(!s(H) || s(it) || !s(Ie))) {
      c(it, !0);
      try {
        const f = s(K).length > 0 ? Math.min(...s(K).map((q) => q.message_id)) : 0, g = await _e("telegram", "telegram_list_media", {
          chatId: s(H).id,
          chatType: s(H).chat_type,
          mediaType: s(Z) === "all" ? null : s(Z),
          offset: f,
          limit: 100
        });
        if (g.length > 0) {
          const q = new Set(s(K).map((B) => B.message_id)), S = g.filter((B) => !q.has(B.message_id));
          c(K, [...s(K), ...S], !0);
        }
        c(Ie, g.length >= 100);
      } catch (f) {
        const g = typeof f == "string" ? f : f.message ?? t()("common.error");
        Bt("error", g);
      } finally {
        c(it, !1);
      }
    }
  }
  async function Un() {
    if (!s(H)) return;
    const f = s(pt).trim();
    if (!f) {
      vr();
      return;
    }
    c(Zt, !0), c(Ce, !0), c(qe, ""), c(Ie, !1);
    try {
      const g = await _e("telegram", "telegram_search_media", {
        chatId: s(H).id,
        chatType: s(H).chat_type,
        query: f,
        mediaType: s(Z) === "all" ? null : s(Z),
        limit: 100
      });
      c(K, g, !0);
    } catch (g) {
      c(qe, typeof g == "string" ? g : g.message ?? t()("telegram.media_error"), !0);
    } finally {
      c(Ce, !1), c(Zt, !1);
    }
  }
  function gs() {
    Et && clearTimeout(Et), Et = setTimeout(
      () => {
        s(pt).trim() ? Un() : (c(Ie, !0), vr());
      },
      400
    );
  }
  function ws(f) {
    c(Z, f, !0), c(xe, /* @__PURE__ */ new Map(), !0), c(De, null), c(ce, 0), c(ke, 0), c(Ie, !0), s(pt).trim() ? Un() : vr();
  }
  function ms(f) {
    return f === 0 ? "—" : f < 1024 ? `${f} B` : f < 1024 * 1024 ? `${(f / 1024).toFixed(1)} KB` : f < 1024 * 1024 * 1024 ? `${(f / (1024 * 1024)).toFixed(1)} MB` : `${(f / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }
  function bs(f) {
    return new Date(f * 1e3).toLocaleDateString();
  }
  function ys(f) {
    const g = `telegram.chat_type_${f}`;
    return t()(g);
  }
  async function Vn() {
    const f = rl();
    if (f != null && f.download.always_ask_path)
      return await Qn({ directory: !0, title: t()("telegram.choose_folder") });
    const g = (f == null ? void 0 : f.download.default_output_dir) ?? null;
    return g || await Qn({ directory: !0, title: t()("telegram.choose_folder") });
  }
  async function qs(f) {
    if (!s(H) || s(Je).has(f.message_id)) return;
    const g = await Vn();
    if (g) {
      c(Je, /* @__PURE__ */ new Set([...s(Je), f.message_id]), !0);
      try {
        const q = await _e("telegram", "telegram_download_media", {
          chatId: s(H).id,
          chatType: s(H).chat_type,
          messageId: f.message_id,
          fileName: f.file_name,
          outputDir: g
        });
        s(Mt).set(q.id, f.message_id), c(Mt, new Map(s(Mt)), !0), Bt("info", t()("toast.download_started", { name: f.file_name }));
      } catch (q) {
        const S = typeof q == "string" ? q : q.message ?? t()("common.error");
        Bt("error", S), c(Je, new Set([...s(Je)].filter((B) => B !== f.message_id)), !0);
      }
    }
  }
  async function xs() {
    if (!s(H) || s(Jt) || s(K).length === 0) return;
    const f = await Vn();
    if (!f) return;
    const g = s(K).map((q) => ({
      message_id: q.message_id,
      file_name: q.file_name,
      file_size: q.file_size
    }));
    c(ke, g.length, !0), c(ce, 0), c(xe, new Map(g.map((q) => [q.message_id, { status: "waiting", percent: 0 }])), !0);
    try {
      const q = await _e("telegram", "telegram_download_batch", {
        chatId: s(H).id,
        chatType: s(H).chat_type,
        chatTitle: s(H).title,
        items: g,
        outputDir: f
      });
      c(De, q, !0);
    } catch (q) {
      const S = typeof q == "string" ? q : q.message ?? t()("common.error");
      Bt("error", S), c(xe, /* @__PURE__ */ new Map(), !0), c(ke, 0);
    }
  }
  async function ks() {
    if (s(De)) {
      try {
        await _e("telegram", "telegram_cancel_batch", { batchId: s(De) }), Bt("info", t()("telegram.batch_cancelled"));
      } catch {
      }
      c(De, null);
    }
  }
  function Es(f) {
    var g;
    return ((g = s(xe).get(f)) == null ? void 0 : g.status) ?? null;
  }
  function Ms(f) {
    var g;
    return ((g = s(xe).get(f)) == null ? void 0 : g.percent) ?? 0;
  }
  function Ts() {
    return Nr < ns ? (Nr++, Promise.resolve()) : new Promise((f) => {
      Lr.push(() => {
        Nr++, f();
      });
    });
  }
  function Ss() {
    Nr--, Lr.length > 0 && Lr.shift()();
  }
  async function As(f, g, q) {
    if (s(He).has(q)) return s(He).get(q);
    const S = Dr;
    await Ts();
    try {
      if (S !== Dr) return null;
      if (s(He).has(q)) return s(He).get(q);
      const B = await _e("telegram", "telegram_get_thumbnail", { chatId: f, chatType: g, messageId: q });
      return S !== Dr ? null : (s(He).set(q, B), c(He, new Map(s(He)), !0), B);
    } catch {
      return null;
    } finally {
      Ss();
    }
  }
  function $r() {
    c(He, /* @__PURE__ */ new Map(), !0), Dr++, Lr.length = 0;
  }
  async function Cs(f, g) {
    if (!s(gt).has(f))
      try {
        const q = await _e("telegram", "telegram_get_chat_photo", { chatId: f, chatType: g });
        s(gt).set(f, q), c(gt, new Map(s(gt)), !0);
      } catch {
      }
  }
  function en(f, g) {
    if (s(gt).has(g.chatId)) return;
    const q = new IntersectionObserver(
      (S) => {
        S[0].isIntersecting && (q.disconnect(), Cs(g.chatId, g.chatType));
      },
      { rootMargin: "200px" }
    );
    return q.observe(f), {
      destroy() {
        q.disconnect();
      }
    };
  }
  function tn(f, g) {
    if (!s(H) || g.mediaType !== "photo" && g.mediaType !== "video" || s(He).has(g.messageId)) return;
    const q = s(H).id, S = s(H).chat_type, B = new IntersectionObserver(
      (ae) => {
        ae[0].isIntersecting && (B.disconnect(), As(q, S, g.messageId));
      },
      { rootMargin: "200px" }
    );
    return B.observe(f), {
      destroy() {
        B.disconnect();
      }
    };
  }
  var Kn = bi(), Is = dn(Kn);
  {
    var Ds = (f) => {
      var g = Ri(), q = M(h(g), 2), S = h(q);
      L((B) => k(S, B), [() => t()("telegram.checking_session")]), E(f, g);
    }, Ns = (f) => {
      var g = Bi(), q = h(g), S = h(q), B = h(S), ae = M(B);
      {
        let U = /* @__PURE__ */ ft(() => t()("hints.telegram"));
        nl(ae, {
          get text() {
            return s(U);
          },
          dismissKey: "telegram"
        });
      }
      var se = M(S, 2);
      {
        var we = (U) => {
          var re = Pi(), Me = M(h(re), 2), Ne = h(Me);
          L((Ve) => k(Ne, Ve), [() => t()("telegram.qr_loading")]), E(U, re);
        }, ve = (U) => {
          var re = Oi(), Me = h(re), Ne = h(Me), Ve = M(Me, 2), Ke = h(Ve);
          L(
            (me) => {
              k(Ne, s(z)), k(Ke, me);
            },
            [() => t()("common.retry")]
          ), be("click", Ve, cr), E(U, re);
        }, J = (U) => {
          var re = Fi();
          Mi(re, () => s(y), !0), E(U, re);
        };
        fe(se, (U) => {
          s(b) ? U(we) : s(z) ? U(ve, 1) : s(y) && U(J, 2);
        });
      }
      var ze = M(se, 2), Ue = h(ze), Ee = h(Ue), $e = M(Ue, 2), j = h($e), G = M(ze, 2), x = M(h(G), 2), I = h(x), C = M(G, 2), Q = h(C);
      L(
        (U, re, Me, Ne, Ve) => {
          k(B, `${U ?? ""} `), k(Ee, re), k(j, Me), k(I, Ne), k(Q, Ve);
        },
        [
          () => t()("telegram.title"),
          () => t()("telegram.qr_title"),
          () => t()("telegram.qr_instruction"),
          () => t()("telegram.or_separator"),
          () => t()("telegram.use_phone")
        ]
      ), be("click", C, os), E(f, g);
    }, Ls = (f) => {
      var g = Hi(), q = h(g), S = h(q), B = h(S), ae = M(S, 2), se = h(ae), we = h(se), ve = h(we), J = M(we, 2), ze = M(J, 2), Ue = h(ze), Ee = M(se, 2);
      {
        var $e = (C) => {
          var Q = ji(), U = h(Q);
          L(() => k(U, s(_))), E(C, Q);
        };
        fe(Ee, (C) => {
          s(_) && C($e);
        });
      }
      var j = M(Ee, 2), G = h(j), x = M(ae, 2), I = h(x);
      L(
        (C, Q, U, re, Me, Ne, Ve) => {
          k(B, C), k(ve, Q), jt(J, "placeholder", U), J.disabled = s(m), k(Ue, re), j.disabled = Me, k(G, Ne), k(I, Ve);
        },
        [
          () => t()("telegram.title"),
          () => t()("telegram.phone_label"),
          () => t()("telegram.phone_placeholder"),
          () => t()("telegram.phone_hint"),
          () => s(m) || !s(i).trim(),
          () => s(m) ? t()("telegram.sending_code") : t()("telegram.send_code"),
          () => t()("telegram.back_to_qr")
        ]
      ), _n("submit", ae, (C) => {
        C.preventDefault(), us();
      }), pr(J, () => s(i), (C) => c(i, C)), be("click", x, fs), E(f, g);
    }, Rs = (f) => {
      var g = Ui(), q = h(g), S = h(q), B = h(S), ae = M(S, 2), se = h(ae), we = h(se), ve = h(we), J = M(we, 2), ze = M(J, 2), Ue = h(ze), Ee = M(se, 2);
      {
        var $e = (x) => {
          var I = zi(), C = h(I);
          L(() => k(C, s(_))), E(x, I);
        };
        fe(Ee, (x) => {
          s(_) && x($e);
        });
      }
      var j = M(Ee, 2), G = h(j);
      L(
        (x, I, C, Q, U, re) => {
          k(B, x), k(ve, I), jt(J, "placeholder", C), J.disabled = s(m), k(Ue, Q), j.disabled = U, k(G, re);
        },
        [
          () => t()("telegram.title"),
          () => t()("telegram.code_label"),
          () => t()("telegram.code_placeholder"),
          () => t()("telegram.code_hint"),
          () => s(m) || !s(u).trim(),
          () => s(m) ? t()("telegram.verifying") : t()("telegram.verify")
        ]
      ), _n("submit", ae, (x) => {
        x.preventDefault(), cs();
      }), pr(J, () => s(u), (x) => c(u, x)), E(f, g);
    }, Ps = (f) => {
      var g = Wi(), q = h(g), S = h(q), B = h(S), ae = M(S, 2), se = h(ae), we = h(se), ve = h(we), J = M(we, 2), ze = M(J, 2);
      {
        var Ue = (x) => {
          var I = Vi(), C = h(I);
          L((Q) => k(C, Q), [
            () => t()("telegram.password_hint", { hint: s(v) })
          ]), E(x, I);
        };
        fe(ze, (x) => {
          s(v) && x(Ue);
        });
      }
      var Ee = M(se, 2);
      {
        var $e = (x) => {
          var I = Ki(), C = h(I);
          L(() => k(C, s(_))), E(x, I);
        };
        fe(Ee, (x) => {
          s(_) && x($e);
        });
      }
      var j = M(Ee, 2), G = h(j);
      L(
        (x, I, C, Q) => {
          k(B, x), k(ve, I), jt(J, "placeholder", C), J.disabled = s(m), j.disabled = s(m) || !s(o), k(G, Q);
        },
        [
          () => t()("telegram.title"),
          () => t()("telegram.password_label"),
          () => t()("telegram.password_placeholder"),
          () => s(m) ? t()("telegram.password_verifying") : t()("telegram.password_submit")
        ]
      ), _n("submit", ae, (x) => {
        x.preventDefault(), vs();
      }), pr(J, () => s(o), (x) => c(o, x)), E(f, g);
    }, Os = (f) => {
      var g = $i(), q = h(g), S = h(q), B = h(S), ae = M(S, 2), se = h(ae), we = h(se);
      let ve;
      var J = M(se, 2), ze = h(J), Ue = M(q, 2);
      {
        var Ee = (x) => {
          var I = Gi(), C = M(h(I), 2), Q = h(C);
          L((U) => k(Q, U), [() => t()("telegram.loading_chats")]), E(x, I);
        }, $e = (x) => {
          var I = Yi(), C = h(I), Q = h(C), U = M(C, 2), re = h(U);
          L(
            (Me) => {
              k(Q, s(ie)), k(re, Me);
            },
            [() => t()("common.retry")]
          ), be("click", U, $t), E(x, I);
        }, j = (x) => {
          var I = Qi(), C = h(I);
          L((Q) => k(C, Q), [() => t()("telegram.no_chats")]), E(x, I);
        }, G = (x) => {
          var I = Ji(), C = dn(I), Q = h(C), U = h(Q), re = M(Q, 2), Me = h(re), Ne = M(C, 2), Ve = M(Ne, 2);
          pn(Ve, 21, () => s(as), (Ke) => Ke.id, (Ke, me) => {
            var dr = Zi(), X = h(dr);
            let N;
            var $ = h(X);
            {
              var et = (Le) => {
                var We = Xi();
                L((_r) => jt(We, "src", `data:image/jpeg;base64,${_r ?? ""}`), [() => s(gt).get(s(me).id)]), E(Le, We);
              }, wt = /* @__PURE__ */ ft(() => s(gt).get(s(me).id)), Te = (Le) => {
                var We = mt();
                L((_r) => k(We, _r), [() => s(me).title.charAt(0).toUpperCase()]), E(Le, We);
              };
              fe($, (Le) => {
                s(wt) ? Le(et) : Le(Te, -1);
              });
            }
            aa(X, (Le, We) => en == null ? void 0 : en(Le, We), () => ({ chatId: s(me).id, chatType: s(me).chat_type }));
            var tt = M(X, 2), er = h(tt), rn = h(er), nn = M(er, 2), an = h(nn);
            L(
              (Le, We) => {
                N = Rr(X, 1, "chat-avatar svelte-1wqcg45", null, N, Le), k(rn, s(me).title), k(an, We);
              },
              [
                () => ({ "has-photo": s(gt).get(s(me).id) }),
                () => ys(s(me).chat_type)
              ]
            ), be("click", dr, () => _s(s(me))), E(Ke, dr);
          }), L(
            (Ke, me) => {
              k(U, Ke), k(Me, me);
            },
            [
              () => t()("telegram.chats_title"),
              () => s(W).length === 1 ? t()("telegram.chat_count_one", { count: s(W).length }) : t()("telegram.chat_count", { count: s(W).length })
            ]
          ), pr(Ne, () => s(ye), (Ke) => c(ye, Ke)), E(x, I);
        };
        fe(Ue, (x) => {
          s(Y) ? x(Ee) : s(ie) ? x($e, 1) : s(W).length === 0 ? x(j, 2) : x(G, -1);
        });
      }
      L(
        (x, I, C) => {
          k(B, x), se.disabled = s(Y), jt(se, "aria-label", I), ve = Rr(we, 0, "svelte-1wqcg45", null, ve, { spinning: s(Y) }), k(ze, C);
        },
        [
          () => t()("telegram.logged_as", { phone: s(d) || "—" }),
          () => t()("hotmart.refresh"),
          () => t()("telegram.logout")
        ]
      ), be("click", se, $t), be("click", J, ds), E(f, g);
    }, Fs = (f) => {
      var g = ko(), q = h(g), S = h(q), B = M(h(S)), ae = M(S, 2), se = h(ae), we = M(q, 2);
      pn(
        we,
        5,
        () => [
          { key: "all", label: t()("telegram.filter_all") },
          { key: "photo", label: t()("telegram.filter_photo") },
          { key: "video", label: t()("telegram.filter_video") },
          { key: "document", label: t()("telegram.filter_document") },
          { key: "audio", label: t()("telegram.filter_audio") }
        ],
        qi,
        (j, G) => {
          var x = eo();
          let I;
          var C = h(x);
          L(() => {
            I = Rr(x, 1, "button filter-btn svelte-1wqcg45", null, I, { active: s(Z) === s(G).key }), x.disabled = s(Jt), k(C, s(G).label);
          }), be("click", x, () => ws(s(G).key)), E(j, x);
        }
      );
      var ve = M(we, 2);
      Li(ve, (j) => c(Pt, j), () => s(Pt));
      var J = M(ve, 2);
      {
        var ze = (j) => {
          var G = to(), x = M(h(G), 2), I = h(x);
          L((C) => k(I, C), [
            () => s(Zt) ? t()("telegram.searching") : t()("telegram.loading_media")
          ]), E(j, G);
        }, Ue = (j) => {
          var G = ro(), x = h(G), I = h(x), C = M(x, 2), Q = h(C);
          L(
            (U) => {
              k(I, s(qe)), k(Q, U);
            },
            [() => t()("common.retry")]
          ), be("click", C, vr), E(j, G);
        }, Ee = (j) => {
          var G = no(), x = h(G);
          L((I) => k(x, I), [() => t()("telegram.no_media")]), E(j, G);
        }, $e = (j) => {
          var G = xo(), x = dn(G), I = h(x), C = h(I), Q = M(I, 2), U = h(Q);
          {
            var re = (X) => {
              var N = ao(), $ = h(N);
              L((et) => k($, et), [() => t()("telegram.cancel_batch")]), be("click", N, ks), E(X, N);
            }, Me = (X) => {
              var N = so(), $ = h(N);
              L(
                (et) => {
                  N.disabled = s(K).length === 0, k($, et);
                },
                [() => t()("telegram.download_all")]
              ), be("click", N, xs), E(X, N);
            };
            fe(U, (X) => {
              s(Jt) ? X(re) : X(Me, -1);
            });
          }
          var Ne = M(x, 2);
          {
            var Ve = (X) => {
              var N = lo(), $ = h(N), et = h($), wt = M($, 2), Te = h(wt);
              L(
                (tt) => {
                  Ai(et, `width: ${s(rs) ?? ""}%`), k(Te, tt);
                },
                [
                  () => t()("telegram.batch_progress", { done: s(ce), total: s(ke) })
                ]
              ), E(X, N);
            };
            fe(Ne, (X) => {
              s(ke) > 0 && X(Ve);
            });
          }
          var Ke = M(Ne, 2);
          pn(Ke, 21, () => s(K), (X) => X.message_id, (X, N) => {
            const $ = /* @__PURE__ */ ft(() => Es(s(N).message_id)), et = /* @__PURE__ */ ft(() => Ms(s(N).message_id));
            var wt = bo(), Te = h(wt);
            let tt;
            var er = h(Te);
            {
              var rn = (T) => {
                var R = io();
                L((de) => jt(R, "src", `data:image/jpeg;base64,${de ?? ""}`), [() => s(He).get(s(N).message_id)]), E(T, R);
              }, nn = /* @__PURE__ */ ft(() => (s(N).media_type === "photo" || s(N).media_type === "video") && s(He).get(s(N).message_id)), an = (T) => {
                var R = vo(), de = h(R);
                {
                  var sn = (oe) => {
                    var Ge = oo();
                    E(oe, Ge);
                  }, ln = (oe) => {
                    var Ge = fo();
                    E(oe, Ge);
                  }, on = (oe) => {
                    var Ge = uo();
                    E(oe, Ge);
                  }, ot = (oe) => {
                    var Ge = co();
                    E(oe, Ge);
                  };
                  fe(de, (oe) => {
                    s(N).media_type === "photo" ? oe(sn) : s(N).media_type === "video" ? oe(ln, 1) : s(N).media_type === "audio" ? oe(on, 2) : oe(ot, -1);
                  });
                }
                E(T, R);
              };
              fe(er, (T) => {
                s(nn) ? T(rn) : T(an, -1);
              });
            }
            var Le = M(Te, 2), We = h(Le), _r = h(We), Bs = M(We, 2), Wn = h(Bs), js = M(Wn);
            {
              var Hs = (T) => {
                var R = mt();
                L((de) => k(R, `· ${de ?? ""}%`), [() => Math.round(s(et))]), E(T, R);
              }, zs = (T) => {
                var R = mt();
                L((de) => k(R, `· ${de ?? ""}`), [() => t()("telegram.downloaded")]), E(T, R);
              }, Us = (T) => {
                var R = mt();
                L((de) => k(R, `· ${de ?? ""}`), [() => t()("telegram.status_skipped")]), E(T, R);
              }, Vs = (T) => {
                var R = mt();
                L((de) => k(R, `· ${de ?? ""}`), [() => t()("telegram.status_error")]), E(T, R);
              }, Ks = (T) => {
                var R = mt();
                L((de) => k(R, `· ${de ?? ""}`), [() => t()("telegram.status_waiting")]), E(T, R);
              };
              fe(js, (T) => {
                s($) === "downloading" ? T(Hs) : s($) === "done" ? T(zs, 1) : s($) === "skipped" ? T(Us, 2) : s($) === "error" ? T(Vs, 3) : s($) === "waiting" && T(Ks, 4);
              });
            }
            var Ws = M(Le, 2);
            {
              var Gs = (T) => {
                var R = _o();
                E(T, R);
              }, Ys = (T) => {
                var R = ho();
                E(T, R);
              }, Qs = (T) => {
                var R = po();
                E(T, R);
              }, Xs = (T) => {
                var R = go();
                E(T, R);
              }, Zs = (T) => {
                var R = wo();
                E(T, R);
              }, Js = (T) => {
                var R = mo(), de = h(R);
                {
                  var sn = (ot) => {
                    const oe = /* @__PURE__ */ ft(() => s(Ot).get(s(N).message_id) ?? 0);
                    var Ge = mt();
                    L(($s) => k(Ge, $s), [
                      () => s(oe) > 0 ? `${Math.round(s(oe))}%` : t()("telegram.downloading")
                    ]), E(ot, Ge);
                  }, ln = /* @__PURE__ */ ft(() => s(Je).has(s(N).message_id)), on = (ot) => {
                    var oe = mt();
                    L((Ge) => k(oe, Ge), [() => t()("telegram.download_btn")]), E(ot, oe);
                  };
                  fe(de, (ot) => {
                    s(ln) ? ot(sn) : ot(on, -1);
                  });
                }
                L((ot) => R.disabled = ot, [
                  () => s(Je).has(s(N).message_id) || s(Jt)
                ]), be("click", R, () => qs(s(N))), E(T, R);
              };
              fe(Ws, (T) => {
                s($) === "downloading" ? T(Gs) : s($) === "done" ? T(Ys, 1) : s($) === "skipped" ? T(Qs, 2) : s($) === "error" ? T(Xs, 3) : s($) === "waiting" ? T(Zs, 4) : T(Js, -1);
              });
            }
            aa(wt, (T, R) => tn == null ? void 0 : tn(T, R), () => ({
              messageId: s(N).message_id,
              mediaType: s(N).media_type
            })), L(
              (T, R, de) => {
                tt = Rr(Te, 1, "media-icon svelte-1wqcg45", null, tt, T), k(_r, s(N).file_name), k(Wn, `${R ?? ""} · ${de ?? ""} `);
              },
              [
                () => ({
                  "has-thumb": (s(N).media_type === "photo" || s(N).media_type === "video") && s(He).get(s(N).message_id)
                }),
                () => ms(s(N).file_size),
                () => bs(s(N).date)
              ]
            ), E(X, wt);
          });
          var me = M(Ke, 2);
          {
            var dr = (X) => {
              var N = qo(), $ = h(N);
              {
                var et = (Te) => {
                  var tt = yo();
                  E(Te, tt);
                }, wt = (Te) => {
                  var tt = mt();
                  L((er) => k(tt, er), [() => t()("telegram.load_more")]), E(Te, tt);
                };
                fe($, (Te) => {
                  s(it) ? Te(et) : Te(wt, -1);
                });
              }
              L(() => N.disabled = s(it) || s(Jt)), be("click", N, ps), E(X, N);
            };
            fe(me, (X) => {
              s(Ie) && X(dr);
            });
          }
          L((X) => k(C, X), [
            () => t()("telegram.file_count", { count: s(K).length })
          ]), E(j, G);
        };
        fe(J, (j) => {
          s(Ce) ? j(ze) : s(qe) ? j(Ue, 1) : s(K).length === 0 ? j(Ee, 2) : j($e, -1);
        });
      }
      L(
        (j, G) => {
          k(B, ` ${j ?? ""}`), k(se, s(H).title), jt(ve, "placeholder", G), ve.disabled = s(Jt);
        },
        [
          () => t()("telegram.back_to_chats"),
          () => t()("telegram.search_files")
        ]
      ), be("click", S, hs), be("input", ve, gs), pr(ve, () => s(pt), (j) => c(pt, j)), E(f, g);
    };
    fe(Is, (f) => {
      s(l) === "checking" ? f(Ds) : s(l) === "qr" ? f(Ns, 1) : s(l) === "phone" ? f(Ls, 2) : s(l) === "code" ? f(Rs, 3) : s(l) === "password" ? f(Ps, 4) : s(l) === "chats" ? f(Os, 5) : s(l) === "media" && s(H) && f(Fs, 6);
    });
  }
  E(e, Kn), Nl(), a();
}
pi(["click", "input"]);
export {
  Ro as TelegramPage
};
