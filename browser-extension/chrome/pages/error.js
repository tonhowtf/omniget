import { resolveErrorPageContent } from "./error-content.js";

const params = new URLSearchParams(window.location.search);

const code = params.get("code") ?? "LAUNCH_FAILED";
const message = params.get("message") ?? "";
const url = params.get("url") ?? "";
const installUrl = params.get("installUrl") ?? "https://github.com/tonhowtf/omniget/releases/latest";

const eyebrow = document.getElementById("eyebrow");
const title = document.getElementById("title");
const body = document.getElementById("body");
const detail = document.getElementById("detail");
const urlNode = document.getElementById("url");
const installLink = document.getElementById("install-link");
const openExtensionsBtn = document.getElementById("open-extensions");

const content = resolveErrorPageContent({ code, message });

document.title = content.documentTitle;
document.documentElement.lang = chrome.i18n?.getUILanguage?.() ?? "en";
eyebrow.textContent = content.eyebrow;
title.textContent = content.title;
body.textContent = content.body;
detail.textContent = content.detail;
urlNode.textContent = url;
installLink.href = installUrl;
installLink.textContent = content.installLabel;
openExtensionsBtn.textContent = content.openExtensionsLabel;

openExtensionsBtn.addEventListener("click", () => {
  chrome.tabs.create({ url: "chrome://extensions" });
});
