import test from "node:test";
import assert from "node:assert/strict";

import { isContextMenuId, linksInText, urlsForMenuClick } from "../src/context-menu.js";

test("the three entries are ours, nothing else is", () => {
  assert.ok(isContextMenuId("omniget-download"));
  assert.ok(isContextMenuId("omniget-download-page"));
  assert.ok(isContextMenuId("omniget-download-selection"));
  assert.ok(!isContextMenuId("other-extension-item"));
});

test("a link or media entry sends that URL, a page entry sends the page", () => {
  assert.deepEqual(
    urlsForMenuClick({ menuItemId: "omniget-download", linkUrl: "https://example.com/v/1" }),
    ["https://example.com/v/1"],
  );
  assert.deepEqual(
    urlsForMenuClick({ menuItemId: "omniget-download", srcUrl: "https://cdn.example.com/a.mp4" }),
    ["https://cdn.example.com/a.mp4"],
  );
  assert.deepEqual(
    urlsForMenuClick({ menuItemId: "omniget-download-page", pageUrl: "https://video.blender.org/w/abc" }),
    ["https://video.blender.org/w/abc"],
  );
  assert.deepEqual(urlsForMenuClick({ menuItemId: "omniget-download", srcUrl: "blob:https://x/1" }), []);
  assert.deepEqual(urlsForMenuClick({ menuItemId: "omniget-download-page", pageUrl: "chrome://extensions" }), []);
});

test("a selection sends every link written in it, once each", () => {
  const text = "Watch https://youtu.be/aqz-KE-bpKQ, then (https://en.wikipedia.org/wiki/Foo_(bar)) and www.example.com/x.\nAgain: https://youtu.be/aqz-KE-bpKQ";
  assert.deepEqual(urlsForMenuClick({ menuItemId: "omniget-download-selection", selectionText: text }), [
    "https://youtu.be/aqz-KE-bpKQ",
    "https://en.wikipedia.org/wiki/Foo_(bar)",
    "https://www.example.com/x",
  ]);
});

test("a selection without links sends nothing, and a long one is capped", () => {
  assert.deepEqual(linksInText("no links here"), []);
  assert.deepEqual(linksInText(undefined), []);
  const many = Array.from({ length: 30 }, (_, i) => `https://example.com/${i}`).join(" ");
  assert.equal(linksInText(many).length, 20);
});
