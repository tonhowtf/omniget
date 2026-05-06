import {
  loadBridgeConfig,
  saveBridgeConfig,
  checkBridgeHealth,
  discoverBridgeEndpoint,
  trimEndpoint,
} from "../src/bridge-client.js";

const endpointInput = document.getElementById("endpoint");
const tokenInput = document.getElementById("token");
const revealBtn = document.getElementById("reveal");
const form = document.getElementById("pair-form");
const testBtn = document.getElementById("test");
const statusEl = document.getElementById("status");
const welcomeHeader = document.getElementById("welcome-header");
const settingsHeader = document.getElementById("settings-header");
const discoveryEl = document.getElementById("discovery-status");
const discoveryMessage = document.getElementById("discovery-message");
const endpointHint = document.getElementById("endpoint-hint");
const advancedDetails = document.getElementById("advanced");

const FALLBACK_ENDPOINT = "http://127.0.0.1:47720";

function resolvedEndpoint() {
  return trimEndpoint(endpointInput.value) || FALLBACK_ENDPOINT;
}

function setStatus(message, kind) {
  statusEl.textContent = message ?? "";
  statusEl.classList.remove("ok", "error");
  if (kind === "ok") statusEl.classList.add("ok");
  if (kind === "error") statusEl.classList.add("error");
}

function setDiscovery(state, message) {
  discoveryEl.classList.remove("found", "missing");
  if (state === "found") discoveryEl.classList.add("found");
  if (state === "missing") discoveryEl.classList.add("missing");
  discoveryMessage.textContent = message;
}

async function init() {
  const { endpoint, token } = await loadBridgeConfig();
  const alreadyPaired = Boolean(token);

  // Show the welcome heading on first run, the regular settings heading
  // once the user is already paired (they're here to inspect / change).
  if (alreadyPaired) {
    settingsHeader.hidden = false;
  } else {
    welcomeHeader.hidden = false;
  }

  endpointInput.value = endpoint || "";
  tokenInput.value = token || "";

  // Skip auto-discovery if the user is already paired AND we have a stored
  // endpoint that responds — they're probably here to change the token, no
  // need to overwrite the URL they trust.
  if (alreadyPaired) {
    const result = await checkBridgeHealth(endpoint);
    if (result.ok) {
      const versionSuffix = result.version ? ` (v${result.version})` : "";
      setDiscovery("found", `Connected to OmniGet${versionSuffix} at ${endpoint}.`);
      return;
    }
    setDiscovery(
      "missing",
      `Couldn't reach the saved endpoint ${endpoint}. Probing default ports…`
    );
  }

  const found = await discoverBridgeEndpoint();
  if (found) {
    endpointInput.value = found.endpoint;
    const versionSuffix = found.version ? ` (v${found.version})` : "";
    setDiscovery(
      "found",
      `Found OmniGet${versionSuffix} on ${found.endpoint}. Paste the token from OmniGet → Settings → Network → Browser extension to finish.`
    );
    endpointHint.textContent =
      "Auto-detected — change only if your OmniGet runs on a different host.";
    return;
  }

  // Discovery failed: open the Advanced disclosure so the user can supply
  // the endpoint manually.
  if (advancedDetails) advancedDetails.open = true;
  setDiscovery(
    "missing",
    "OmniGet doesn't seem to be running. Launch the desktop app, then refresh this page — or set the endpoint manually below."
  );
}

revealBtn.addEventListener("click", () => {
  const next = tokenInput.type === "password" ? "text" : "password";
  tokenInput.type = next;
  revealBtn.textContent = next === "password" ? "Show" : "Hide";
  revealBtn.setAttribute("aria-pressed", String(next !== "password"));
});

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  const endpoint = resolvedEndpoint();
  const token = tokenInput.value.trim();
  if (!token) {
    setStatus("Paste the pairing token first.", "error");
    return;
  }
  await saveBridgeConfig({ endpoint, token });
  setStatus("Saved. The extension will use this token from now on.", "ok");
});

testBtn.addEventListener("click", async () => {
  const endpoint = resolvedEndpoint();
  setStatus("Testing connection…");
  const result = await checkBridgeHealth(endpoint);
  if (result.ok) {
    const versionSuffix = result.version ? ` (v${result.version})` : "";
    setStatus(`Connected to OmniGet${versionSuffix} at ${endpoint}.`, "ok");
  } else {
    setStatus(
      `Could not reach OmniGet at ${endpoint}. Make sure the app is running.`,
      "error"
    );
  }
});

init();
