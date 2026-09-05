// Forçar H.264 no YouTube (estudo 39, enhanced-h264ify — MIT). Roda no
// "mundo" da página, antes de qualquer script do YouTube: faz o navegador
// dizer que não sabe tocar VP9/AV1 (e, se pedido, 60 fps), então o player
// escolhe H.264, que máquinas fracas decodificam por hardware.
(() => {
  const script = document.currentScript;
  const block60 = script?.dataset?.block60 === "1";
  const blocked = [/vp9|vp09|vp8|vp08|av01/i];
  const reject = (type) => {
    if (typeof type !== "string") return false;
    if (blocked.some((re) => re.test(type))) return true;
    if (block60) {
      const m = type.match(/framerate=(\d+)/);
      if (m && Number(m[1]) > 30) return true;
    }
    return false;
  };
  const wrap = (obj, name) => {
    const orig = obj[name];
    if (typeof orig !== "function") return;
    obj[name] = function (type, ...rest) {
      if (reject(type)) return name === "canPlayType" ? "" : false;
      return orig.call(this, type, ...rest);
    };
  };
  if (window.MediaSource) wrap(window.MediaSource, "isTypeSupported");
  if (window.HTMLMediaElement?.prototype) wrap(window.HTMLMediaElement.prototype, "canPlayType");
  if (window.MediaCapabilities?.prototype) {
    const orig = window.MediaCapabilities.prototype.decodingInfo;
    if (typeof orig === "function") {
      window.MediaCapabilities.prototype.decodingInfo = function (cfg) {
        const type = cfg?.video?.contentType;
        if (reject(type) || (block60 && cfg?.video?.framerate > 30)) {
          return Promise.resolve({ supported: false, smooth: false, powerEfficient: false });
        }
        return orig.call(this, cfg);
      };
    }
  }
})();
