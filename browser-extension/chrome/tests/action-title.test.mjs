import test from "node:test";
import assert from "node:assert/strict";

import { resolveActionTitle } from "../src/action-title.js";

test("uses localized title for supported pages", () => {
  const title = resolveActionTitle(true, (key) =>
    key === "action_title_supported" ? "Envoyer cette page multimedia vers OmniGet" : ""
  );

  assert.equal(title, "Envoyer cette page multimedia vers OmniGet");
});

test("uses localized title for unsupported pages", () => {
  const title = resolveActionTitle(false, (key) =>
    key === "action_title_unsupported" ? "Aucun media pris en charge detecte sur cette page" : ""
  );

  assert.equal(title, "Aucun media pris en charge detecte sur cette page");
});

test("falls back to English when a locale key is missing", () => {
  assert.equal(resolveActionTitle(true, () => ""), "Send this media page to OmniGet");
  assert.equal(resolveActionTitle(false, () => ""), "No supported media detected on this page");
});
