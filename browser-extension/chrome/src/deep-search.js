// Portions adapted from cat-catch (catch-script/search.js)
// Copyright (c) xifangczy — https://github.com/xifangczy/cat-catch
// Licensed under GPL-3.0, same as this project.
//
// Deep search runs in the page's MAIN world and watches the JavaScript APIs a
// player uses to build a playlist that never crosses the network with an
// honest Content-Type: fetch, XMLHttpRequest, JSON.parse, TextDecoder and
// Worker. Whatever it finds is handed to the isolated-world bridge through
// `window.postMessage`; this file never talks to `chrome.*` and never touches
// the DOM.
//
// Every hook is wrapped in try/catch. A throw of ours inside a page hook would
// break the site, so in the worst case each hook must behave exactly as if it
// were not installed.
(function omnigetDeepSearch() {
  "use strict";

  const MESSAGE_SOURCE = "omniget-deep-search";
  const SYNTHETIC_ORIGIN = "https://deep-search.omniget.invalid";
  const MESSAGE_KIND = "manifest";

  // Globals the worker prelude sets so a hook running inside a worker still
  // knows which page it belongs to and where its script was loaded from.
  const PAGE_URL_GLOBAL = "__omnigetDeepSearchPageUrl";
  const WORKER_BASE_GLOBAL = "__omnigetDeepSearchWorkerBase";
  const INSTALLED_GLOBAL = "__omnigetDeepSearchInstalled";

  // Reading a body costs memory and CPU on every single response, which is why
  // catching "everything" makes an extension that freezes heavy sites. Bodies
  // are only read when the Content-Type could plausibly carry a manifest and
  // the payload is small.
  const READABLE_CONTENT_TYPE = /(mpegurl|dash\+xml|json|text\/plain|octet-stream|xml)/i;
  const MAX_BODY_BYTES = 4 * 1024 * 1024;

  // A manifest marker can only sit at the very top of the payload, so a large
  // blob of unrelated text is rejected after a fixed-size scan.
  const MAX_CLASSIFY_SCAN = 4096;

  // Cheap pre-filter before walking a parsed JSON tree. One indexOf-style test
  // over the raw text is orders of magnitude cheaper than a recursive walk, and
  // JSON.parse is hot on most pages.
  const MANIFEST_HINT = /#EXTM3U|\.m3u8|\.m3u(?![a-z0-9])|\.mpd/i;
  const JSON_SCAN_MAX_DEPTH = 8;
  const JSON_SCAN_MAX_NODES = 4000;

  const URI_ATTRIBUTE = /URI="([^"]*)"/gi;
  const MEDIA_URL_PATH = /\.(m3u8|m3u|mpd)(?:$|[?#])/i;

  const MAX_REPORTED = 256;

  // ---------------------------------------------------------------- pure part

  function classifyManifest(text) {
    if (typeof text !== "string" || text.length === 0) return null;
    let head = text.length > MAX_CLASSIFY_SCAN ? text.slice(0, MAX_CLASSIFY_SCAN) : text;
    if (head.charCodeAt(0) === 0xfeff) head = head.slice(1);
    head = head.replace(/^\s+/, "");
    if (head.length === 0) return null;
    if (head.slice(0, 7).toUpperCase() === "#EXTM3U") return "hls";
    if (head.charAt(0) === "<") {
      if (/<MPD[\s>]/i.test(head)) return "dash";
      if (/urn:mpeg:dash:schema:mpd/i.test(head)) return "dash";
    }
    return null;
  }

  function parseContentLength(value) {
    if (value === null || value === undefined || value === "") return null;
    const size = typeof value === "number" ? value : parseInt(String(value), 10);
    if (!Number.isFinite(size) || size < 0) return null;
    return size;
  }

  function shouldReadBody(contentType, contentLength) {
    if (typeof contentType !== "string" || contentType === "") return false;
    if (!READABLE_CONTENT_TYPE.test(contentType)) return false;
    const size = parseContentLength(contentLength);
    // A missing or unparseable Content-Length is allowed through: the read is
    // capped at MAX_BODY_BYTES instead of trusting the header.
    if (size === null) return true;
    return size <= MAX_BODY_BYTES;
  }

  function resolveUrl(value, base) {
    if (typeof value !== "string" || value.trim() === "") return null;
    try {
      return new URL(value, base).href;
    } catch {
      return null;
    }
  }

  function absolutizeLine(line, base) {
    const trimmed = line.trim();
    if (trimmed === "") return line;
    if (trimmed.charAt(0) === "#") {
      if (trimmed.indexOf("URI=") === -1) return line;
      return line.replace(URI_ATTRIBUTE, (match, value) => {
        const absolute = resolveUrl(value, base);
        return absolute === null ? match : `URI="${absolute}"`;
      });
    }
    const absolute = resolveUrl(trimmed, base);
    if (absolute === null || absolute === trimmed) return line;
    // Replacing the trimmed slice keeps any surrounding whitespace byte for
    // byte; the function form of replace avoids `$&` surprises in the URL.
    return line.replace(trimmed, () => absolute);
  }

  function absolutizeManifest(text, baseUrl) {
    if (typeof text !== "string" || text === "") return text;
    let base;
    try {
      base = new URL(baseUrl);
    } catch {
      return text;
    }
    // The capturing group keeps the original line terminators in the array, so
    // a CRLF playlist comes back out as CRLF.
    const parts = text.split(/(\r\n|\n|\r)/);
    for (let i = 0; i < parts.length; i += 2) {
      parts[i] = absolutizeLine(parts[i], base);
    }
    return parts.join("");
  }

  function randomId() {
    try {
      const uuid = globalThis.crypto?.randomUUID?.();
      if (typeof uuid === "string") return uuid.replace(/-/g, "");
    } catch {
      // fall through to the arithmetic fallback
    }
    let id = "";
    while (id.length < 32) {
      id += Math.random().toString(16).slice(2);
    }
    return id.slice(0, 32);
  }

  // A playlist that only ever existed as a `blob:` URL cannot be fetched from
  // outside the tab, so it gets a synthetic http(s) identity instead. The URL
  // is never requested: the native side consumes the text we ship alongside it.
  // A playlist that only ever existed as a `blob:` has no address the desktop
  // app could fetch, so it gets an identity instead of a location: the app
  // downloads from the text we send alongside it, and this URL is never
  // requested by anyone.
  //
  // The host is a reserved `.invalid` name (RFC 2606) rather than the page's
  // own host, for two reasons. It can never resolve, so a bug that does try to
  // fetch it fails loudly instead of hitting the site. And the app picks its
  // downloader by host — a synthetic URL on `youtube.com` would be handed to
  // the YouTube extractor, which would ignore the playlist we captured. The
  // page host stays in the path so the URL is still readable in a log.
  function syntheticManifestUrl(pageUrl) {
    let host;
    try {
      const parsed = new URL(pageUrl);
      if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return null;
      host = parsed.host;
    } catch {
      return null;
    }
    if (!host) return null;
    return `${SYNTHETIC_ORIGIN}/${encodeURIComponent(host)}/${randomId()}.m3u8`;
  }

  function hashText(text) {
    let hash = 5381;
    for (let i = 0; i < text.length; i++) {
      hash = ((hash << 5) + hash + text.charCodeAt(i)) | 0;
    }
    return (hash >>> 0).toString(36) + ":" + text.length;
  }

  function toHttpUrl(value) {
    if (typeof value !== "string" || value === "") return null;
    try {
      const parsed = new URL(value);
      if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return null;
      return parsed.href;
    } catch {
      return null;
    }
  }

  const isWorkerScope =
    typeof WorkerGlobalScope !== "undefined" &&
    typeof self !== "undefined" &&
    self instanceof WorkerGlobalScope;

  // Exposed for the node test suite only. In a real page `window` exists, so
  // nothing is ever attached to the page's global object.
  if (typeof window === "undefined" && !isWorkerScope) {
    globalThis.__omnigetDeepSearchInternals = {
      absolutizeManifest,
      classifyManifest,
      shouldReadBody,
      syntheticManifestUrl,
      parseContentLength,
      hashText,
      toHttpUrl,
      MAX_BODY_BYTES,
    };
    return;
  }

  // ------------------------------------------------------------ install part

  // Re-registering the content script (or an iframe that reuses this realm)
  // must not wrap the same hook twice. The marker is non-enumerable so page
  // code enumerating globals never trips over it.
  try {
    if (self[INSTALLED_GLOBAL]) return;
    Object.defineProperty(self, INSTALLED_GLOBAL, {
      value: true,
      enumerable: false,
      configurable: true,
      writable: true,
    });
  } catch {
    // A frozen global is not a reason to skip the hooks.
  }

  const nativeJsonParse = JSON.parse;
  const nativeTextDecode = typeof TextDecoder === "function" ? TextDecoder.prototype.decode : null;
  const nativeXhrOpen = typeof XMLHttpRequest === "function" ? XMLHttpRequest.prototype.open : null;
  const nativeFetch = typeof self.fetch === "function" ? self.fetch : null;
  const NativeWorker = typeof self.Worker === "function" ? self.Worker : null;
  const nativePostMessage = typeof self.postMessage === "function" ? self.postMessage : null;

  const reported = new Set();
  const syntheticUrls = new Map();
  // Raw playlist text -> whether it was already reported with a real http URL.
  // The same playlist is routinely seen twice (the fetch hook sees the body,
  // then the page decodes it again through TextDecoder), and only the sighting
  // that carries a real URL is worth a second message.
  const seenText = new Map();

  function pageHref() {
    try {
      const injected = self[PAGE_URL_GLOBAL];
      if (typeof injected === "string" && injected !== "") return injected;
    } catch {
      // ignore
    }
    try {
      return location.href;
    } catch {
      return "";
    }
  }

  function remember(key) {
    if (reported.has(key)) return false;
    reported.add(key);
    if (reported.size > MAX_REPORTED) {
      const oldest = reported.values().next().value;
      reported.delete(oldest);
    }
    return true;
  }

  function deliver(message) {
    try {
      if (!remember(message.format + "|" + message.url)) return;
      if (isWorkerScope) {
        // Inside a worker there is no window: the page-side Worker wrapper
        // listens for this message and re-posts it to the page.
        if (nativePostMessage) nativePostMessage.call(self, message);
        return;
      }
      // Same-window postMessage. An opaque origin (a sandboxed frame) has no
      // matching targetOrigin, so the message is simply dropped there.
      window.postMessage(message, location.origin);
    } catch {
      // ignore
    }
  }

  function reportManifest(text, sourceUrl) {
    const format = classifyManifest(text);
    if (!format) return;
    const httpUrl = toHttpUrl(sourceUrl);
    if (format === "dash") {
      // The native side only knows how to consume HLS playlist text today, so
      // a DASH manifest is reported by URL only and never with its body.
      // Shipping the MPD text would advertise support that does not exist.
      if (!httpUrl) return;
      deliver({
        source: MESSAGE_SOURCE,
        kind: MESSAGE_KIND,
        url: httpUrl,
        format: "dash",
        pageUrl: pageHref(),
      });
      return;
    }
    const rawKey = hashText(text);
    const previous = seenText.get(rawKey);
    if (previous === true || (previous === false && !httpUrl)) return;
    seenText.set(rawKey, Boolean(httpUrl));
    if (seenText.size > MAX_REPORTED) {
      const oldest = seenText.keys().next().value;
      seenText.delete(oldest);
    }

    const page = pageHref();
    const base = httpUrl || page;
    let absolute;
    try {
      absolute = absolutizeManifest(text, base);
    } catch {
      absolute = text;
    }
    let url = httpUrl;
    if (!url) {
      // Keep one synthetic identity per distinct playlist text so the same
      // blob playlist is not reported again on every re-parse.
      const key = hashText(absolute);
      url = syntheticUrls.get(key) || null;
      if (!url) {
        url = syntheticManifestUrl(page);
        if (!url) return;
        syntheticUrls.set(key, url);
      }
    }
    deliver({
      source: MESSAGE_SOURCE,
      kind: MESSAGE_KIND,
      url,
      text: absolute,
      format: "hls",
      pageUrl: page,
    });
  }

  function reportMediaUrl(value) {
    const httpUrl = toHttpUrl(value);
    if (!httpUrl) return;
    let path;
    try {
      path = new URL(httpUrl).pathname;
    } catch {
      return;
    }
    if (!MEDIA_URL_PATH.test(path)) return;
    deliver({
      source: MESSAGE_SOURCE,
      kind: MESSAGE_KIND,
      url: httpUrl,
      format: /\.mpd$/i.test(path) ? "dash" : "hls",
      pageUrl: pageHref(),
    });
  }

  function scanJson(value, depth, budget) {
    if (budget.nodes <= 0) return;
    budget.nodes--;
    if (typeof value === "string") {
      if (value.length > MAX_BODY_BYTES) return;
      if (classifyManifest(value) === "hls") {
        reportManifest(value, null);
        return;
      }
      if (MEDIA_URL_PATH.test(value)) reportMediaUrl(value);
      return;
    }
    if (!value || typeof value !== "object") return;
    if (depth >= JSON_SCAN_MAX_DEPTH) return;
    if (Array.isArray(value)) {
      for (let i = 0; i < value.length; i++) {
        if (budget.nodes <= 0) return;
        scanJson(value[i], depth + 1, budget);
      }
      return;
    }
    for (const key in value) {
      if (budget.nodes <= 0) return;
      try {
        scanJson(value[key], depth + 1, budget);
      } catch {
        // A getter that throws is the page's business, not ours.
      }
    }
  }

  function scanJsonRoot(value) {
    scanJson(value, 0, { nodes: JSON_SCAN_MAX_NODES });
  }

  function handleBodyText(text, sourceUrl, contentType) {
    if (typeof text !== "string" || text === "") return;
    if (text.length > MAX_BODY_BYTES) return;
    if (classifyManifest(text)) {
      reportManifest(text, sourceUrl);
      return;
    }
    if (!MANIFEST_HINT.test(text)) return;
    const first = text.charAt(text.search(/\S/));
    if (first !== "{" && first !== "[") return;
    if (typeof contentType === "string" && contentType !== "" && !/json|text\/plain|octet-stream/i.test(contentType)) {
      return;
    }
    let parsed;
    try {
      parsed = nativeJsonParse(text);
    } catch {
      return;
    }
    scanJsonRoot(parsed);
  }

  // ------------------------------------------------------------------- hooks

  // JSON.parse — the cheapest way to see a playlist a site embeds in an API
  // payload. The raw-text pre-filter keeps the common case to one regex test.
  JSON.parse = function (text) {
    const result = nativeJsonParse.apply(this, arguments);
    try {
      if (typeof text === "string" && text.length <= MAX_BODY_BYTES && MANIFEST_HINT.test(text)) {
        scanJsonRoot(result);
      }
    } catch {
      // ignore
    }
    return result;
  };
  JSON.parse.toString = function () {
    return nativeJsonParse.toString();
  };

  // fetch — the body is only read when the Content-Type and Content-Length say
  // it could be a manifest. Cloning and reading every response is exactly what
  // makes a deep-search extension unusable on heavy sites.
  if (nativeFetch) {
    self.fetch = function (input, init) {
      const promise = nativeFetch.apply(this, arguments);
      try {
        return promise.then((response) => {
          try {
            inspectResponse(response, input);
          } catch {
            // ignore
          }
          return response;
        });
      } catch {
        return promise;
      }
    };
    self.fetch.toString = function () {
      return nativeFetch.toString();
    };
  }

  function headerOf(response, name) {
    try {
      const headers = response.headers;
      if (!headers || typeof headers.get !== "function") return null;
      return headers.get(name);
    } catch {
      return null;
    }
  }

  function requestUrlOf(input) {
    if (typeof input === "string") return input;
    if (input && typeof input === "object" && typeof input.url === "string") return input.url;
    return "";
  }

  function inspectResponse(response, input) {
    if (!response || response.ok !== true) return;
    const contentType = headerOf(response, "content-type") || "";
    if (!shouldReadBody(contentType, headerOf(response, "content-length"))) return;
    let clone;
    try {
      clone = response.clone();
    } catch {
      return;
    }
    readCappedText(clone)
      .then((text) => {
        if (text === null) return;
        handleBodyText(text, response.url || requestUrlOf(input), contentType);
      })
      .catch(() => {});
  }

  // Streams the body and gives up as soon as it passes the cap. A truncated
  // playlist is worse than no playlist, so an oversized body returns null.
  async function readCappedText(response) {
    const body = response.body;
    if (!body || typeof body.getReader !== "function" || !nativeTextDecode) {
      const text = await response.text();
      return text.length > MAX_BODY_BYTES ? null : text;
    }
    const reader = body.getReader();
    const decoder = new TextDecoder("utf-8");
    let out = "";
    let total = 0;
    for (;;) {
      const chunk = await reader.read();
      if (chunk.done) break;
      total += chunk.value ? chunk.value.byteLength : 0;
      if (total > MAX_BODY_BYTES) {
        try {
          await reader.cancel();
        } catch {
          // ignore
        }
        return null;
      }
      // The native decode is used on purpose: our own TextDecoder hook would
      // otherwise re-enter on every chunk we read ourselves.
      out += nativeTextDecode.call(decoder, chunk.value, { stream: true });
    }
    out += nativeTextDecode.call(decoder);
    return out;
  }

  // XMLHttpRequest — same content-type gate, and the response is only touched
  // for text-ish responseTypes, since reading responseText otherwise throws.
  if (nativeXhrOpen) {
    XMLHttpRequest.prototype.open = function () {
      try {
        this.addEventListener("readystatechange", function () {
          try {
            inspectXhr(this);
          } catch {
            // ignore
          }
        });
      } catch {
        // ignore
      }
      return nativeXhrOpen.apply(this, arguments);
    };
    XMLHttpRequest.prototype.open.toString = function () {
      return nativeXhrOpen.toString();
    };
  }

  function inspectXhr(xhr) {
    if (xhr.readyState !== 4) return;
    if (xhr.status !== 200) return;
    const responseType = xhr.responseType;
    if (responseType !== "" && responseType !== "text") return;
    let contentType = "";
    let contentLength = null;
    try {
      contentType = xhr.getResponseHeader("content-type") || "";
      contentLength = xhr.getResponseHeader("content-length");
    } catch {
      return;
    }
    if (!shouldReadBody(contentType, contentLength)) return;
    let text;
    try {
      text = xhr.responseText;
    } catch {
      return;
    }
    handleBodyText(text, xhr.responseURL, contentType);
  }

  // TextDecoder — catches a playlist assembled from raw bytes in memory, the
  // MSE path where nothing recognisable ever hits the network.
  if (nativeTextDecode) {
    TextDecoder.prototype.decode = function () {
      const result = nativeTextDecode.apply(this, arguments);
      try {
        if (
          typeof result === "string" &&
          result.length >= 7 &&
          result.length <= MAX_BODY_BYTES &&
          classifyManifest(result) === "hls"
        ) {
          reportManifest(result, null);
        }
      } catch {
        // ignore
      }
      return result;
    };
    TextDecoder.prototype.decode.toString = function () {
      return nativeTextDecode.toString();
    };
  }

  // Worker — hls.js and shaka parse the playlist inside a worker, where none of
  // the hooks above exist. The worker source is fetched with a synchronous XHR
  // and re-served from a Blob with this whole script prepended, so the hooks
  // are installed in the worker realm too. Any failure falls straight back to
  // the native Worker: breaking the site's worker is never acceptable.
  if (NativeWorker && !isWorkerScope) {
    const WorkerHook = function (scriptUrl, options) {
      try {
        const patched = buildPatchedWorkerUrl(scriptUrl);
        if (patched) {
          const worker = new NativeWorker(patched, options);
          try {
            worker.addEventListener("message", onWorkerMessage);
          } catch {
            // ignore
          }
          return worker;
        }
      } catch {
        // ignore
      }
      return new NativeWorker(scriptUrl, options);
    };
    WorkerHook.prototype = NativeWorker.prototype;
    WorkerHook.toString = function () {
      return NativeWorker.toString();
    };
    try {
      self.Worker = WorkerHook;
    } catch {
      // ignore
    }
  }

  function buildPatchedWorkerUrl(scriptUrl) {
    if (typeof Blob !== "function" || typeof URL.createObjectURL !== "function") return null;
    if (!nativeXhrOpen) return null;
    if (typeof scriptUrl !== "string" && !(scriptUrl instanceof URL)) return null;
    const href = String(scriptUrl);
    if (href === "") return null;
    let absolute;
    try {
      absolute = href.startsWith("blob:") || href.startsWith("data:") ? href : new URL(href, location.href).href;
    } catch {
      return null;
    }
    const xhr = new XMLHttpRequest();
    // Synchronous on purpose: `new Worker(...)` must return a worker before the
    // caller's next statement, so there is nowhere to await.
    nativeXhrOpen.call(xhr, "GET", absolute, false);
    xhr.send();
    if (xhr.status !== 200 && xhr.status !== 0) return null;
    const source = xhr.responseText;
    if (typeof source !== "string" || source === "") return null;
    const prelude =
      "self." + PAGE_URL_GLOBAL + "=" + JSON.stringify(pageHref()) + ";" +
      "self." + WORKER_BASE_GLOBAL + "=" + JSON.stringify(absolute) + ";" +
      // Serving the worker from a Blob moves its base URL, which would break a
      // relative importScripts. Resolve those against the original script URL.
      "(function(){var i=self.importScripts;if(typeof i!==\"function\")return;" +
      "self.importScripts=function(){var a=[];for(var k=0;k<arguments.length;k++){" +
      "try{a.push(new URL(arguments[k],self." + WORKER_BASE_GLOBAL + ").href);}catch(e){a.push(arguments[k]);}}" +
      "return i.apply(self,a);};})();" +
      "(" + omnigetDeepSearch.toString() + ")();\n";
    const blob = new Blob([prelude, source], { type: "text/javascript" });
    return URL.createObjectURL(blob);
  }

  function onWorkerMessage(event) {
    try {
      const data = event && event.data;
      if (!data || typeof data !== "object") return;
      if (data.source !== MESSAGE_SOURCE || data.kind !== MESSAGE_KIND) return;
      const message = {
        source: MESSAGE_SOURCE,
        kind: MESSAGE_KIND,
        url: data.url,
        format: data.format,
        pageUrl: pageHref(),
      };
      if (typeof data.text === "string") message.text = data.text;
      deliver(message);
    } catch {
      // ignore
    }
  }
})();
