(function (w) {
  function fill(target, name, value) {
    if (target && !(name in target)) {
      Object.defineProperty(target, name, { value: value, writable: true, configurable: true });
    }
  }
  function at(n) {
    var i = Math.trunc(n) || 0;
    if (i < 0) i += this.length;
    return i < 0 || i >= this.length ? undefined : this[i];
  }
  fill(Array.prototype, "at", at);
  fill(String.prototype, "at", at);
  if (w.Int8Array) fill(Object.getPrototypeOf(w.Int8Array.prototype), "at", at);
  fill(Array.prototype, "findLast", function (fn, thisArg) {
    for (var i = this.length - 1; i >= 0; i--) if (fn.call(thisArg, this[i], i, this)) return this[i];
    return undefined;
  });
  fill(Array.prototype, "findLastIndex", function (fn, thisArg) {
    for (var i = this.length - 1; i >= 0; i--) if (fn.call(thisArg, this[i], i, this)) return i;
    return -1;
  });
  fill(Object, "hasOwn", function (o, k) {
    return Object.prototype.hasOwnProperty.call(o, k);
  });
  fill(Object, "groupBy", function (items, fn) {
    var out = Object.create(null);
    var i = 0;
    Array.from(items).forEach(function (item) {
      var k = fn(item, i++);
      (out[k] = out[k] || []).push(item);
    });
    return out;
  });
  fill(w.Promise, "withResolvers", function () {
    var r = {};
    r.promise = new Promise(function (resolve, reject) {
      r.resolve = resolve;
      r.reject = reject;
    });
    return r;
  });
  fill(w, "structuredClone", function (value) {
    function copy(v, seen) {
      if (v === null || typeof v !== "object") return v;
      if (seen.has(v)) return seen.get(v);
      if (v instanceof Date) return new Date(v.getTime());
      if (v instanceof RegExp) return new RegExp(v.source, v.flags);
      if (ArrayBuffer.isView(v) || v instanceof ArrayBuffer) return v.slice(0);
      var out;
      if (Array.isArray(v)) {
        out = [];
        seen.set(v, out);
        v.forEach(function (x, i) {
          out[i] = copy(x, seen);
        });
        return out;
      }
      if (v instanceof Map) {
        out = new Map();
        seen.set(v, out);
        v.forEach(function (x, k) {
          out.set(copy(k, seen), copy(x, seen));
        });
        return out;
      }
      if (v instanceof Set) {
        out = new Set();
        seen.set(v, out);
        v.forEach(function (x) {
          out.add(copy(x, seen));
        });
        return out;
      }
      out = {};
      seen.set(v, out);
      Object.keys(v).forEach(function (k) {
        out[k] = copy(v[k], seen);
      });
      return out;
    }
    return copy(value, new Map());
  });
  if (w.crypto) {
    fill(w.crypto, "randomUUID", function () {
      var b = w.crypto.getRandomValues(new Uint8Array(16));
      b[6] = (b[6] & 0x0f) | 0x40;
      b[8] = (b[8] & 0x3f) | 0x80;
      var h = Array.from(b, function (x) {
        return (x + 256).toString(16).slice(1);
      }).join("");
      return [h.slice(0, 8), h.slice(8, 12), h.slice(12, 16), h.slice(16, 20), h.slice(20)].join("-");
    });
  }
  if (w.HTMLElement && !("showModal" in w.document.createElement("dialog"))) {
    var proto = w.HTMLElement.prototype;
    fill(proto, "showModal", function () {
      this.setAttribute("open", "");
    });
    fill(proto, "show", function () {
      this.setAttribute("open", "");
    });
    fill(proto, "close", function (result) {
      if (!this.hasAttribute("open")) return;
      this.removeAttribute("open");
      if (result !== undefined) this.returnValue = result;
      this.dispatchEvent(new Event("close"));
    });
    if (!("open" in proto)) {
      Object.defineProperty(proto, "open", {
        configurable: true,
        get: function () {
          return this.hasAttribute("open");
        },
        set: function (v) {
          if (v) this.setAttribute("open", "");
          else this.removeAttribute("open");
        },
      });
    }
    var style = w.document.createElement("style");
    style.textContent =
      "dialog:not([open]){display:none!important}dialog[open]{display:block;position:fixed;top:0;right:0;bottom:0;left:0;margin:auto;z-index:2147483000}";
    w.document.head.appendChild(style);
  }
})(window);
