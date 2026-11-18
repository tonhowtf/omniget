import { describe, expect, it } from "vitest";
import { installDom } from "../test-dom";
import { sanitizeMarkup } from "./foliate";

const hasDom = await installDom();

const XHTML = "application/xhtml+xml";

function xhtml(body: string): string {
  return `<?xml version="1.0" encoding="UTF-8"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:svg="http://www.w3.org/2000/svg" xmlns:x="http://www.w3.org/1999/xlink"><head><title>t</title></head><body>${body}</body></html>`;
}

describe.skipIf(!hasDom)("sanitizeMarkup", () => {
  it("removes an img error handler", () => {
    const out = sanitizeMarkup('<p><img src="a.png" onerror="alert(1)"></p>', "text/html");
    expect(out).not.toContain("onerror");
    expect(out).toContain('src="a.png"');
  });

  it("removes a javascript href", () => {
    const out = sanitizeMarkup('<p><a href=" java\tscript:alert(1)">x</a></p>', "text/html");
    expect(out).not.toMatch(/script:alert/);
    expect(out).toContain(">x</a>");
  });

  it("removes a javascript href hidden behind a custom xlink prefix", () => {
    const out = sanitizeMarkup(xhtml('<svg:svg><svg:a x:href="javascript:alert(1)"><svg:text>go</svg:text></svg:a></svg:svg>'), XHTML);
    expect(out).not.toContain("javascript:");
    expect(out).toContain("go");
  });

  it("removes a javascript xlink:href", () => {
    const out = sanitizeMarkup(
      '<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><a xlink:href="javascript:alert(1)"><text>go</text></a></svg>',
      "image/svg+xml",
    );
    expect(out).not.toContain("javascript:");
  });

  it("removes animation elements that rewrite href", () => {
    for (const tag of ["set", "animate", "animateTransform"]) {
      const out = sanitizeMarkup(
        xhtml(`<svg:svg><svg:a href="#"><svg:${tag} attributeName="xlink:href" to="javascript:alert(1)"/><svg:text>go</svg:text></svg:a></svg:svg>`),
        XHTML,
      );
      expect(out).not.toContain("javascript:");
      expect(out).not.toContain("attributeName");
    }
  });

  it("keeps animation elements that do not touch href", () => {
    const out = sanitizeMarkup(xhtml('<svg:svg><svg:rect><svg:set attributeName="fill" to="red"/></svg:rect></svg:svg>'), XHTML);
    expect(out).toContain('attributeName="fill"');
  });

  it("removes scripts, frames and the book's own http-equiv meta", () => {
    const out = sanitizeMarkup(
      '<html><head><meta http-equiv="refresh" content="0;url=https://evil.example"><script>alert(1)</script></head><body><iframe src="x"></iframe><p>ok</p></body></html>',
      "text/html",
    );
    expect(out).not.toContain("<script");
    expect(out).not.toContain("<iframe");
    expect(out).not.toContain("refresh");
    expect(out).toContain("<p>ok</p>");
  });

  it("puts a script-blocking policy first in the head of html and xhtml chapters", () => {
    for (const out of [sanitizeMarkup("<p>hi</p>", "text/html"), sanitizeMarkup(xhtml("<p>hi</p>"), XHTML)]) {
      expect(out).toMatch(/<head[^>]*>\s*<meta[^>]*http-equiv="Content-Security-Policy"[^>]*content="script-src 'none'"/);
    }
  });

  it("falls back to html parsing for malformed xhtml and still cleans it", () => {
    const out = sanitizeMarkup('<html><body><p onclick="alert(1)">a<br>b</p></body></html>', XHTML);
    expect(out).not.toContain("onclick");
    expect(out).toContain("Content-Security-Policy");
  });
});
