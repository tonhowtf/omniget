// Portions adapted from cat-catch (catch-script/search.js)
// Copyright (c) xifangczy — https://github.com/xifangczy/cat-catch
// Licensed under GPL-3.0, same as this project.
//
// Isolated-world half of deep search. It is the only side that can talk to
// `chrome.*`, so it takes the manifests the MAIN world posts, keeps their text
// in this frame's memory, and tells the background that a playlist exists.
//
// The text itself never goes over the extension message channel: a playlist can
// be hundreds of kilobytes and the background has no use for it until the user
// actually asks to download. It is fetched back on demand with
// `getManifestText`.
(function omnigetDeepSearchBridge() {
  "use strict";

  const MESSAGE_SOURCE = "omniget-deep-search";
  const MESSAGE_KIND = "manifest";
  const MAX_MANIFESTS = 64;

  if (typeof chrome === "undefined" || !chrome.runtime) return;

  // Insertion-ordered, so the first key is always the oldest entry.
  const manifests = new Map();

  function rememberManifest(url, text) {
    if (typeof url !== "string" || url === "") return;
    if (typeof text !== "string" || text === "") return;
    // Re-inserting moves the entry to the end, keeping it away from eviction.
    if (manifests.has(url)) manifests.delete(url);
    manifests.set(url, text);
    while (manifests.size > MAX_MANIFESTS) {
      const oldest = manifests.keys().next().value;
      manifests.delete(oldest);
    }
  }

  function getManifestText(url) {
    if (typeof url !== "string" || url === "") return null;
    const text = manifests.get(url);
    return typeof text === "string" ? text : null;
  }

  function notifyBackground(payload) {
    try {
      const sent = chrome.runtime.sendMessage(payload);
      if (sent && typeof sent.catch === "function") sent.catch(() => {});
    } catch {
      // The service worker may be gone or the context invalidated; a lost
      // notification is never worth throwing inside a page.
    }
  }

  function onWindowMessage(event) {
    // Only messages this window posted to itself, from this exact origin, are
    // ours. Anything from a frame, an opener or a cross-origin sender is data
    // we did not produce and is dropped before it is looked at.
    if (event.source !== window) return;
    if (event.origin !== location.origin) return;
    const data = event.data;
    if (!data || typeof data !== "object") return;
    if (data.source !== MESSAGE_SOURCE) return;
    if (data.kind !== MESSAGE_KIND) return;

    const url = typeof data.url === "string" ? data.url : "";
    if (url === "") return;
    const text = typeof data.text === "string" && data.text !== "" ? data.text : null;
    if (text) rememberManifest(url, text);

    notifyBackground({
      type: "deep-search-media",
      url,
      format: data.format === "dash" ? "dash" : "hls",
      hasManifest: Boolean(text),
      textLength: text ? text.length : 0,
      pageUrl: typeof data.pageUrl === "string" ? data.pageUrl : location.href,
    });
  }

  try {
    window.addEventListener("message", onWindowMessage);
  } catch {
    // ignore
  }

  try {
    chrome.runtime.onMessage.addListener(function (message, sender, sendResponse) {
      if (!message || message.type !== "getManifestText") return undefined;
      try {
        sendResponse({ text: getManifestText(message.url) });
      } catch {
        // ignore
      }
      // The answer comes straight out of memory, but `true` keeps the channel
      // valid for callers that expect an asynchronous reply.
      return true;
    });
  } catch {
    // ignore
  }
})();
