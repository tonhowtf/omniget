const snifferToggle = document.getElementById("sniffer-toggle");
const pageAction = document.getElementById("page-action");
const sendPageBtn = document.getElementById("send-page");
const platformName = document.getElementById("page-platform");
const mediaList = document.getElementById("media-list");
const emptyState = document.getElementById("empty-state");
const status = document.getElementById("status");

let currentData = null;

function init() {
  chrome.runtime.sendMessage({ type: "getDetectedMedia" }, (response) => {
    if (!response) return;
    currentData = response;

    snifferToggle.checked = response.snifferEnabled;

    if (response.pageDetected?.supported) {
      pageAction.classList.remove("hidden");
      platformName.textContent = response.pageDetected.platform;
    }

    renderMediaList(response.media);
  });

  snifferToggle.addEventListener("change", () => {
    chrome.runtime.sendMessage({
      type: "toggleSniffer",
      enabled: snifferToggle.checked,
    });
  });

  sendPageBtn.addEventListener("click", () => {
    if (!currentData?.tabUrl) return;
    sendToApp(currentData.tabUrl, currentData.pageDetected?.platform);
  });
}

function renderMediaList(media) {
  if (!media || media.length === 0) {
    emptyState.classList.remove("hidden");
    return;
  }

  emptyState.classList.add("hidden");

  for (const entry of media) {
    const item = document.createElement("div");
    item.className = "media-item";

    const icon = getMediaIcon(entry.mediaType);
    const filename = getFilenameFromUrl(entry.url);
    const size = entry.sizeText || "";
    const type = entry.mediaType.toUpperCase();

    item.innerHTML = `
      <div class="media-info">
        <span class="media-icon">${icon}</span>
        <div class="media-details">
          <span class="media-name" title="${escapeHtml(entry.url)}">${escapeHtml(filename)}</span>
          <span class="media-meta">${escapeHtml(type)}${size ? " \u00b7 " + escapeHtml(size) : ""}</span>
        </div>
      </div>
      <button class="download-btn" title="Send to OmniGet">\u2193</button>
    `;

    item.querySelector(".download-btn").addEventListener("click", () => {
      sendToApp(entry.url, "generic", entry);
    });

    mediaList.insertBefore(item, emptyState);
  }
}

function sendToApp(url, platform, mediaEntry) {
  status.textContent = "Sending...";

  const msg = { type: "sendToOmniGet", url, platform };

  if (mediaEntry) {
    const refererHeader = mediaEntry.requestHeaders?.find(
      h => h.name.toLowerCase() === "referer"
    );
    if (refererHeader) msg.referer = refererHeader.value;
  }

  chrome.runtime.sendMessage(msg, (response) => {
    if (response?.ok) {
      status.textContent = "\u2713 Sent!";
      setTimeout(() => window.close(), 800);
    } else {
      status.textContent = "\u2717 Failed \u2014 is OmniGet running?";
    }
  });
}

function getMediaIcon(type) {
  switch (type) {
    case "hls": return "\ud83d\udce1";
    case "dash": return "\ud83d\udce1";
    case "video": return "\ud83c\udfac";
    case "audio": return "\ud83c\udfb5";
    default: return "\ud83d\udce6";
  }
}

function getFilenameFromUrl(url) {
  try {
    const path = new URL(url).pathname;
    const parts = path.split("/");
    const last = parts[parts.length - 1];
    if (last && last.length > 0 && last.includes(".")) {
      return decodeURIComponent(last).substring(0, 50);
    }
    return url.substring(0, 60) + "...";
  } catch {
    return url.substring(0, 60) + "...";
  }
}

function escapeHtml(str) {
  const div = document.createElement("div");
  div.textContent = str;
  return div.innerHTML;
}

init();
