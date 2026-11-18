import { describe, expect, it } from "vitest";
import { renderSafeMarkdownSync } from "./markdown";
import { installDom } from "./test-dom";

function render(text: string, cache = new Map<string, string>(), opts = {}): Promise<string> {
  return new Promise((resolve) => {
    renderSafeMarkdownSync(text, cache, () => resolve(cache.get(text) ?? ""), opts);
  });
}

describe("renderSafeMarkdownSync cache", () => {
  it("returns the rendered html on the second call", async () => {
    const cache = new Map<string, string>();
    const html = await render("hello", cache);
    expect(renderSafeMarkdownSync("hello", cache)).toBe(html);
  });

  it("keeps at most 8 entries and drops the oldest first", async () => {
    const cache = new Map<string, string>();
    for (let i = 0; i < 12; i++) await render(`note ${i}`, cache);
    expect(cache.size).toBe(8);
    expect(cache.has("note 3")).toBe(false);
    expect(cache.has("note 4")).toBe(true);
    expect(cache.has("note 11")).toBe(true);
  });
});

const hasDom = await installDom();

describe.skipIf(!hasDom)("sanitizeHtml through a real DOM", () => {
  it("drops an img with an error handler when images are off", async () => {
    const html = await render('before <img src="x" onerror="alert(1)"> after');
    expect(html).not.toContain("<img");
    expect(html).not.toContain("onerror");
  });

  it("keeps an allowed image but never its handlers", async () => {
    const html = await render('<img src="https://example.com/a.png" alt="pic" onerror="alert(1)">', new Map(), {
      images: true,
    });
    expect(html).toContain('src="https://example.com/a.png"');
    expect(html).toContain('alt="pic"');
    expect(html).not.toContain("onerror");
  });

  it("removes a javascript href and keeps the link text", async () => {
    const html = await render('<a href="javascript:alert(1)">click</a>');
    expect(html).not.toContain("javascript:");
    expect(html).toContain("click");
  });

  it("removes a javascript href written in markdown", async () => {
    const html = await render("[click](javascript:alert(1))");
    expect(html).not.toContain("javascript:");
  });

  it("keeps an https link and hardens it", async () => {
    const html = await render("[site](https://example.com)");
    expect(html).toContain('href="https://example.com"');
    expect(html).toContain('rel="noopener noreferrer"');
    expect(html).toContain('target="_blank"');
  });

  it("removes script and style with their content", async () => {
    const html = await render("<script>alert(1)</script><style>p{color:red}</style>text");
    expect(html).not.toContain("alert(1)");
    expect(html).not.toContain("color:red");
    expect(html).toContain("text");
  });

  it("strips handlers and styles from allowed tags", async () => {
    const html = await render('<div onclick="alert(1)" style="position:fixed">x</div>');
    expect(html).not.toContain("onclick");
    expect(html).not.toContain("style");
  });

  it("unwraps svg animation that targets href", async () => {
    const html = await render('<svg><a><set attributeName="href" to="javascript:alert(1)"/><text>go</text></a></svg>');
    expect(html).not.toContain("<set");
    expect(html).not.toContain("<svg");
    expect(html).not.toContain("attributeName");
  });
});
