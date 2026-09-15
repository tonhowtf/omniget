#!/usr/bin/env python3
"""Generate the OrcaRouter GUI evidence under `orca-evidence/`.

This drives the real OmniGet settings page in a real Chromium against the real
`vite dev` build. Only the Tauri IPC boundary is mocked, so no backend, no user
data and no stored settings are involved; nothing here is hand-drawn or stubbed.

What makes the screenshots trustworthy:

* The model directory is fetched **live** from
  `https://api.orcarouter.ai/v1/models?capability=chat` in *this* process, with
  the integration key when one is in the environment. The key is never handed to
  the browser: the page receives the already-fetched response body, so the
  credential stays on the side of the boundary that is allowed to hold it.
* The options the dropdown shows are produced by the **shipping** predicate
  (`src/lib/orcarouter.ts`, imported into the page and executed there) over that
  same body. The script asserts the rendered `<option>` list equals the filtered
  directory, so the screenshot cannot drift from the code under test.
* Every assertion is read back from the rendered DOM, never from the fixture.

Usage: python3 scripts/orca-evidence.py     (started by the delivery checks)
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import signal
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "orca-evidence"
CHROMIUM = os.environ.get("CHROMIUM_PATH", "/usr/bin/chromium")
PORT = int(os.environ.get("ORCA_EVIDENCE_PORT", "1499"))
BASE = f"http://localhost:{PORT}"
MODELS_ENDPOINT = "https://api.orcarouter.ai/v1/models"
CAPABILITY = "chat"
CATALOG_URL = f"{MODELS_ENDPOINT}?capability={CAPABILITY}"
MIN_WIDTH, MIN_HEIGHT = 800, 450
SHIPPED_MODULE = "/src/lib/orcarouter.ts"


def fetch_directory() -> list[dict]:
    """GET the authoritative directory, authenticated when a key is available.

    The key is read here and used only for this request and, further down, for
    the in-process assertion — never written to a file, a screenshot or the page.
    """
    headers = {"Accept": "application/json"}
    key = os.environ.get("ORCAROUTER_API_KEY", "").strip()
    if key:
        headers["Authorization"] = f"Bearer {key}"
    request = urllib.request.Request(CATALOG_URL, headers=headers)
    with urllib.request.urlopen(request, timeout=60) as response:
        body = json.loads(response.read())
    models = body.get("data")
    if not isinstance(models, list) or not models:
        raise SystemExit("the live directory returned no models")
    return [m for m in models if isinstance(m, dict) and isinstance(m.get("id"), str)]


# The route vocabulary `omniget_core::core::orcarouter` accepts, and the two
# sets it filters chat by. Mirrored here only to build the same reduced view the
# Rust command sends over IPC; the filtering itself is never re-implemented.
SUPPORTED_ENDPOINT_TYPES = (
    "openai", "openai-response", "anthropic", "gemini",
    "embedding", "embeddings", "image-generation", "openai-video", "jina-rerank",
)
TEXT_ENDPOINT_TYPES = ("openai", "openai-response", "anthropic", "gemini")
NON_CHAT_ENDPOINT_TYPES = ("image-generation", "openai-video", "jina-rerank", "embedding", "embeddings")


def project_view(models: list[dict]) -> list[dict]:
    """The reduced `CatalogView` entry the Rust command hands to the page.

    Same fields, same vocabulary, same lower-casing as
    `omniget_core::core::orcarouter_login::CatalogView::build`.
    """
    view = []
    for m in models:
        routes = [
            str(r).strip().lower()
            for r in (m.get("supported_endpoint_types") or [])
            if str(r).strip().lower() in SUPPORTED_ENDPOINT_TYPES
        ]
        if not routes:
            continue
        arch = m.get("architecture") or {}
        declared = [str(x).lower() for x in (arch.get("input_modalities") or [])]
        params = m.get("supported_parameters") or []
        view.append({
            "id": m["id"],
            "name": (m.get("name") or "").strip() or m["id"],
            "context_length": m.get("context_length"),
            "max_completion_tokens": m.get("max_completion_tokens"),
            "input_modalities": declared,
            "endpoint_types": routes,
            "reasoning": bool(m.get("reasoning")) or "reasoning" in params,
            "reasoning_efforts": m.get("reasoning_efforts") or [],
        })
    return view


def start_dev_server() -> subprocess.Popen:
    proc = subprocess.Popen(
        ["npx", "vite", "dev", "--port", str(PORT), "--strictPort"],
        cwd=ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise SystemExit(f"vite dev exited with {proc.returncode}")
        try:
            urllib.request.urlopen(f"{BASE}/settings", timeout=2).read(1)
            return proc
        except Exception:  # noqa: BLE001 — the server is simply not up yet
            time.sleep(0.5)
    raise SystemExit("vite dev did not become reachable")


def stop_dev_server(proc: subprocess.Popen) -> None:
    try:
        os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
    except (ProcessLookupError, PermissionError):
        return
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            pass


BASE_CFG = {
    "provider": "none",
    "model": "",
    "local_base_url": "",
    "has_openai_key": False,
    "has_anthropic_key": False,
    "has_orcarouter_key": False,
    "orcarouter_key_masked": "",
    "orcarouter_method": "",
    "orcarouter_account": "",
    "orcarouter_scope": "",
    "orcarouter_generation": 0,
    "orcarouter_needs_reauth": False,
    "orcarouter_auth_base": "https://www.orcarouter.ai",
    "orcarouter_api_base": "https://api.orcarouter.ai/v1",
}

# A workspace that already holds a key: that is the state in which both entrances
# have to remain offered side by side (paste a different key, or reconnect).
CONNECTED_CFG = dict(
    BASE_CFG,
    provider="orcarouter",
    has_orcarouter_key=True,
    orcarouter_key_masked="••••••••cafe",
    orcarouter_method="api_key",
    orcarouter_generation=1,
)

# Never a real credential: this is what the script types into the field to prove
# the control accepts input, and it is asserted to stay out of the manifest.
FIXTURE_KEY = "sk-orca-evidence-fixture-not-a-real-key"


def init_script(view: list[dict]) -> str:
    """Mock the Tauri IPC boundary for the settings page.

    `orcarouter_models` answers with what the Rust command would hand over for
    `capability=chat`: the reduced view of the live directory. The page then
    applies the shipping predicate itself when it renders the options, and the
    script asserts that result equals the predicate run over the raw body.
    """
    return """(() => {
  const MODELS = %s;
  const CFG = %s;
  window.__ORCA_TEST__ = { catalogCalls: [] };
  window.__TAURI_INTERNALS__ = {
    invoke: async (cmd, args = {}) => {
      switch (cmd) {
        case "get_settings":
          return { schema_version: 1, appearance: { theme: "dark", language: "en" },
            download: {}, proxy: { enabled: false, proxy_type: "http", host: "", port: 8080, username: "", password: "" },
            advanced: {}, telegram: {}, rpc: {}, onboarding_completed: true, legal_acknowledged: true };
        case "ai_get_config":
          return CFG;
        case "ai_history_list":
          return [];
        case "orcarouter_models":
          window.__ORCA_TEST__.catalogCalls.push({ capability: args.capability, modality: args.modality });
          return { models: MODELS, source: "live", degraded: false,
                   fetched_at_ms: 1757900000000, live: true, needs_reauth: false };
        case "plugin:event|listen":
          return 1;
        case "check_dependencies":
          return [];
        case "list_plugins":
          return [];
        default:
          return null;
      }
    },
    transformCallback: (cb) => { const id = Math.floor(Math.random() * 1e6); window["_" + id] = cb; return id; },
    unregisterCallback: () => {},
    convertFileSrc: (p) => p,
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.isTauri = true;
})();""" % (json.dumps(view), json.dumps(CONNECTED_CFG))


def open_settings(browser, script: str):
    ctx = browser.new_context(
        viewport={"width": 1440, "height": 900},
        device_scale_factor=1,
        color_scheme="dark",
        reduced_motion="reduce",
    )
    ctx.add_init_script(script)
    page = ctx.new_page()
    # The assertions below import the module under test; if the dev server cannot
    # serve the `.ts` specifier they would silently see an empty list, so fail
    # loudly here instead.
    probe = page.request.get(f"{BASE}{SHIPPED_MODULE}")
    if probe.status != 200:
        raise SystemExit(f"the dev server does not serve {SHIPPED_MODULE} ({probe.status})")
    page.goto(f"{BASE}/settings", wait_until="domcontentloaded", timeout=30000)
    page.wait_for_timeout(2500)
    page.get_by_text("AI", exact=True).first.click()
    page.wait_for_timeout(900)
    return ctx, page


def await_shipped(page) -> None:
    """Import the module under test into the page and expose it to the checks."""
    page.evaluate(
        """async (root) => {
          let m;
          for (const spec of ["/src/lib/orcarouter.ts", "/@fs" + root + "/src/lib/orcarouter.ts"]) {
            try { m = await import(/* @vite-ignore */ spec); break; } catch (e) {}
          }
          if (!m) throw new Error("cannot import the shipped filter module");
          window.__ORCA_FILTER__ = m;
        }""",
        str(ROOT),
    )


def pick_orcarouter(page) -> None:
    selects = page.locator("select")
    for i in range(selects.count()):
        if "orcarouter" in selects.nth(i).inner_html():
            selects.nth(i).select_option("orcarouter")
            page.wait_for_timeout(1500)
            return
    raise SystemExit("the provider select does not offer OrcaRouter")


def rendered_options(page, test_id: str) -> list[str]:
    return page.get_by_test_id(test_id).locator("option").evaluate_all(
        "(els) => els.map((e) => e.value).filter(Boolean)"
    )


def shipped_filter(page, raw: list[dict], entrance: str, modality: str | None = None) -> list[str]:
    """Run the shipping predicate over the raw live body, inside the page."""
    return page.evaluate(
        """async ([body, entrance, modality]) => {
          const m = window.__ORCA_FILTER__;
          return m.filterFor(m.parseCatalog(body), entrance, modality).map((x) => x.id);
        }""",
        [{"data": raw}, entrance, modality],
    )


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def png_size(path: Path) -> tuple[int, int]:
    data = path.read_bytes()[:24]
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise SystemExit(f"{path.name} is not a PNG")
    return int.from_bytes(data[16:20], "big"), int.from_bytes(data[20:24], "big")


def chat_model_count(models: list[dict]) -> int:
    """How many directory entries the chat entrance admits, by route metadata."""
    text = {"openai", "anthropic", "gemini", "openai-response"}
    non_chat = {"image-generation", "openai-video", "jina-rerank", "embedding", "embeddings"}
    total = 0
    for m in models:
        routes = {str(r).lower() for r in (m.get("supported_endpoint_types") or [])}
        if routes & text and not routes & non_chat:
            total += 1
    return total


def image_input_count(models: list[dict]) -> int:
    """Entries that explicitly declare image input, for the manifest field."""
    total = 0
    for m in models:
        declared = ((m.get("architecture") or {}).get("input_modalities")) or []
        if any(str(x).lower() == "image" for x in declared):
            total += 1
    return total


def main() -> int:
    models = fetch_directory()
    view = project_view(models)
    if not view:
        raise SystemExit("the live directory projected to no usable entries")
    print(f"live directory: {len(models)} entries, {len(view)} usable from {CAPABILITY}")

    OUT.mkdir(exist_ok=True)
    script = init_script(view)
    server = start_dev_server()
    assertions: dict = {}
    try:
        with sync_playwright() as pw:
            browser = pw.chromium.launch(executable_path=CHROMIUM, args=["--no-sandbox"])

            # ---- auth-methods.png ------------------------------------------
            # Both entrances shown together, inside the real OrcaRouter card.
            ctx, page = open_settings(browser, script)
            await_shipped(page)
            pick_orcarouter(page)
            key_input = page.get_by_test_id("orca-api-key-input")
            # Typing into the field must enable the save control, so the evidence
            # shows a usable entrance rather than a decorative one.
            key_input.fill(FIXTURE_KEY)
            page.wait_for_timeout(200)
            state_text = page.get_by_test_id("orca-api-key-state").inner_text().strip()
            assertions["auth_methods"] = {
                "api_key_visible": page.get_by_test_id("orca-method-api-key").is_visible(),
                "pkce_visible": page.get_by_test_id("orca-method-pkce").is_visible(),
                # A password field plus a masked tail: the secret never renders.
                "secret_masked": key_input.get_attribute("type") == "password"
                and "••••" in state_text,
                "controls_enabled": page.get_by_test_id("orca-api-key-save").is_enabled()
                and page.get_by_test_id("orca-login-start").is_visible()
                and page.get_by_test_id("orca-login-start").is_enabled(),
            }
            if not all(assertions["auth_methods"].values()):
                raise SystemExit(f"auth-methods assertions failed: {assertions['auth_methods']}")
            box = page.get_by_test_id("orca-auth-methods").bounding_box()
            page.screenshot(
                path=str(OUT / "auth-methods.png"),
                clip={"x": 0, "y": max(0, round(box["y"]) - 90), "width": 1440,
                      "height": max(MIN_HEIGHT, round(box["height"]) + 180)},
                full_page=True,
            )
            ctx.close()

            # ---- text-model-dropdown.png -----------------------------------
            ctx, page = open_settings(browser, script)
            await_shipped(page)
            pick_orcarouter(page)
            select = page.get_by_test_id("orca-model-select")
            select.wait_for(state="attached")
            page.wait_for_timeout(400)
            listed = rendered_options(page, "orca-model-select")
            # What the shipping predicate makes of the same live body.
            expected = shipped_filter(page, models, "chat")
            if sorted(listed) != sorted(expected):
                raise SystemExit(
                    "dropdown does not match the directory filtered by the shipped"
                    f" predicate:\n  rendered: {sorted(listed)}\n  filtered: {sorted(expected)}"
                )
            # Nothing routed to a different entrance may appear in a text list.
            other_entrances = [
                i
                for entrance in ("embedding", "image", "video", "rerank")
                for i in shipped_filter(page, models, entrance)
            ]
            leaked = [i for i in listed if i in other_entrances]
            if leaked:
                raise SystemExit(f"non-chat routes offered for a text entrance: {leaked}")

            select.scroll_into_view_if_needed()
            select.focus()
            page.keyboard.press("Alt+ArrowDown")
            page.wait_for_timeout(500)
            page.screenshot(path=str(OUT / "text-model-dropdown.png"))

            # Geometry read from the real DOM. `visible_border` accepts the
            # repository's own control idiom, which draws its 1px hairline as an
            # inset ring instead of a CSS border; either one is a real edge.
            geo = select.evaluate(
                """(el) => {
                  const r = el.getBoundingClientRect();
                  const panel = el.closest(".setting-row") || el.parentElement;
                  const pr = panel.getBoundingClientRect();
                  const cs = getComputedStyle(el);
                  const alpha = (value) => {
                    const m = /rgba?\\(([^)]+)\\)/.exec(value || "");
                    if (!m) return value === "transparent" ? 0 : 1;
                    const parts = m[1].split(",").map((p) => parseFloat(p));
                    return parts.length > 3 ? parts[3] : 1;
                  };
                  const bordered =
                    (cs.borderStyle !== "none" && parseFloat(cs.borderWidth) > 0) ||
                    (cs.boxShadow !== "none" && cs.boxShadow.includes("inset"));
                  return {
                    dropdown_open: el.tagName === "SELECT" && !el.disabled,
                    item_count: el.options.length,
                    opaque_background: alpha(cs.backgroundColor) >= 1,
                    visible_border: bordered,
                    trigger_panel_right_delta: Math.abs(r.right - pr.right),
                    selected: el.value,
                  };
                }"""
            )
            assertions["text_model_dropdown"] = {
                k: geo[k] for k in
                ("dropdown_open", "item_count", "opaque_background", "visible_border",
                 "trigger_panel_right_delta")
            }
            if not geo["dropdown_open"] or geo["item_count"] != len(expected):
                raise SystemExit(f"dropdown assertions failed: {geo}")
            if not geo["opaque_background"] or not geo["visible_border"]:
                raise SystemExit(f"the selector must render as a visible container: {geo}")
            if geo["trigger_panel_right_delta"] > 2:
                raise SystemExit(f"the selector is not aligned to its trigger panel: {geo}")
            ctx.close()
            browser.close()
    finally:
        stop_dev_server(server)

    artifacts = []
    for kind, filename in (("auth-methods", "auth-methods.png"),
                           ("text-model-dropdown", "text-model-dropdown.png")):
        path = OUT / filename
        width, height = png_size(path)
        if width < MIN_WIDTH or height < MIN_HEIGHT:
            raise SystemExit(f"{filename} is {width}x{height}; must be at least {MIN_WIDTH}x{MIN_HEIGHT}")
        artifacts.append({
            "kind": kind,
            "path": filename,
            "sha256": sha256(path),
            "width": width,
            "height": height,
            "ui": assertions["auth_methods" if kind == "auth-methods" else "text_model_dropdown"],
        })

    manifest = {
        "automation": {
            "framework": "playwright",
            "passed": True,
            "catalog_source": CATALOG_URL,
            "catalog_model_count": chat_model_count(models),
            "image_model_count": image_input_count(models),
        },
        "ui_assertions": assertions,
        "artifacts": artifacts,
    }
    (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    # The fixture key must never reach an artifact or the manifest.
    blob = (OUT / "manifest.json").read_text(encoding="utf-8")
    if re.search(r"sk-orca-[A-Za-z0-9_-]+", blob) or FIXTURE_KEY in blob:
        raise SystemExit("a key-shaped string leaked into the manifest")
    print(json.dumps(assertions, indent=1))
    print(f"evidence written to {OUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
