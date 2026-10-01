// Copyright (c) xifangczy — https://github.com/xifangczy/cat-catch
// Licensed under GPL-3.0, same as this project.
(function omnigetDeepSearchBridge() {
  "use strict";

  const MESSAGE_SOURCE = "omniget-deep-search";
  const MESSAGE_KIND = "manifest";
  const MAX_MANIFESTS = 64;

  if (typeof chrome === "undefined" || !chrome.runtime) return;

  const manifests = new Map();

  function rememberManifest(url, text) {
    if (typeof url !== "string" || url === "") return;
    if (typeof text !== "string" || text === "") return;
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
    }
  }

  function onWindowMessage(event) {
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
  }

  try {
    chrome.runtime.onMessage.addListener(function (message, sender, sendResponse) {
      if (!message || message.type !== "getManifestText") return undefined;
      try {
        sendResponse({ text: getManifestText(message.url) });
      } catch {
      }
      return true;
    });
  } catch {
  }
})();
