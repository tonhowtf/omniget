import test from "node:test";
import assert from "node:assert/strict";

import { resolveErrorPageContent } from "../pages/error-content.js";

test("uses localized page strings when locale messages are available", () => {
  const localized = {
    error_page_document_title: "Extension OmniGet",
    error_page_eyebrow: "Extension OmniGet",
    error_page_install_cta: "Installer OmniGet",
    error_page_open_extensions: "Ouvrir les extensions Chrome",
    error_host_missing_title: "Titre localise",
    error_host_missing_body: "Corps localise",
    error_host_missing_detail: "Detail localise",
  };

  const content = resolveErrorPageContent({
    code: "HOST_MISSING",
    getMessage: (name) => localized[name] ?? "",
  });

  assert.equal(content.documentTitle, "Extension OmniGet");
  assert.equal(content.eyebrow, "Extension OmniGet");
  assert.equal(content.installLabel, "Installer OmniGet");
  assert.equal(content.openExtensionsLabel, "Ouvrir les extensions Chrome");
  assert.equal(content.title, "Titre localise");
  assert.equal(content.body, "Corps localise");
  assert.equal(content.detail, "Detail localise");
});

test("falls back to English when locale messages are missing", () => {
  const content = resolveErrorPageContent({
    code: "INVALID_URL",
    getMessage: () => "",
  });

  assert.equal(content.documentTitle, "OmniGet Extension");
  assert.equal(content.title, "This page URL cannot be sent to OmniGet");
  assert.equal(
    content.detail,
    "Try again from a direct video, reel, post, playlist, or course page."
  );
});

test("prefers the query message for detail text when one is provided", () => {
  const content = resolveErrorPageContent({
    code: "LAUNCH_FAILED",
    message: "Desktop launch failed with exit code 5",
    getMessage: () => "",
  });

  assert.equal(content.detail, "Desktop launch failed with exit code 5");
});

test("falls back to LAUNCH_FAILED content for unknown error codes", () => {
  const content = resolveErrorPageContent({
    code: "SOMETHING_NEW",
    getMessage: () => "",
  });

  assert.equal(content.code, "LAUNCH_FAILED");
  assert.equal(content.title, "OmniGet could not be launched from Chrome");
});
