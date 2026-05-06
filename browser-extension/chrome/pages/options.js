import {
  loadBridgeConfig,
  saveBridgeConfig,
  checkBridgeHealth,
  trimEndpoint,
} from "../src/bridge-client.js";

const endpointInput = document.getElementById("endpoint");
const tokenInput = document.getElementById("token");
const revealBtn = document.getElementById("reveal");
const form = document.getElementById("pair-form");
const testBtn = document.getElementById("test");
const statusEl = document.getElementById("status");

function setStatus(message, kind) {
  statusEl.textContent = message ?? "";
  statusEl.classList.remove("ok", "error");
  if (kind === "ok") statusEl.classList.add("ok");
  if (kind === "error") statusEl.classList.add("error");
}

async function init() {
  const { endpoint, token } = await loadBridgeConfig();
  endpointInput.value = endpoint || "";
  tokenInput.value = token || "";
}

revealBtn.addEventListener("click", () => {
  const next = tokenInput.type === "password" ? "text" : "password";
  tokenInput.type = next;
  revealBtn.textContent = next === "password" ? "Show" : "Hide";
  revealBtn.setAttribute("aria-pressed", String(next !== "password"));
});

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  const endpoint = trimEndpoint(endpointInput.value);
  const token = tokenInput.value.trim();
  if (!endpoint) {
    setStatus("Endpoint URL is required.", "error");
    return;
  }
  if (!token) {
    setStatus("Token is required.", "error");
    return;
  }
  await saveBridgeConfig({ endpoint, token });
  setStatus("Saved. The extension will use this token from now on.", "ok");
});

testBtn.addEventListener("click", async () => {
  const endpoint = trimEndpoint(endpointInput.value);
  if (!endpoint) {
    setStatus("Enter an endpoint URL first.", "error");
    return;
  }
  setStatus("Testing connection…");
  const result = await checkBridgeHealth(endpoint);
  if (result.ok) {
    const versionSuffix = result.version ? ` (v${result.version})` : "";
    setStatus(`Connected to OmniGet${versionSuffix}.`, "ok");
  } else {
    setStatus(
      `Could not reach OmniGet at ${endpoint}. Make sure the app is running.`,
      "error"
    );
  }
});

init();
