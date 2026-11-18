import { describe, expect, it } from "vitest";
import { definitionsHtml, plainDefinitionHtml } from "./dictionary";

describe("plainDefinitionHtml", () => {
  it("keeps the allowed tags without any attribute", () => {
    expect(plainDefinitionHtml('<b onpointerenter="alert(1)">bold</b> <i class="x" style="y">it</i>')).toBe("<b>bold</b> <i>it</i>");
  });

  it("removes every other tag and keeps its text", () => {
    expect(plainDefinitionHtml('<a href="javascript:alert(1)">link</a><img src=x onerror=alert(1)><span>s</span>')).toBe("links");
  });

  it("does not let a longer tag name pass as an allowed one", () => {
    expect(plainDefinitionHtml("<iframe src=x></iframe><body onload=x><link rel=x><object>")).toBe("");
  });

  it("drops script and style together with their content", () => {
    expect(plainDefinitionHtml("a<script>alert(1)</script>b<style>p{}</style>c")).toBe("abc");
  });

  it("escapes an unclosed tag", () => {
    expect(plainDefinitionHtml('text <img src=x onerror="alert(1)"')).toBe('text &lt;img src=x onerror="alert(1)"');
  });

  it("keeps closing tags and list structure", () => {
    expect(plainDefinitionHtml('<OL start="3"><li data-x="1">one</li></OL>')).toBe("<ol><li>one</li></ol>");
  });

  it("is not fooled by a quoted angle bracket inside an allowed tag", () => {
    const out = plainDefinitionHtml('<b title=">" onpointerenter="alert(1)">x</b>');
    expect(out).not.toMatch(/<[^>]*onpointerenter/);
    expect(out.startsWith("<b>")).toBe(true);
  });
});

describe("definitionsHtml", () => {
  it("renders part of speech and definitions", () => {
    expect(definitionsHtml([{ partOfSpeech: "Noun", definitions: [{ definition: "A <b>thing</b>." }] }])).toBe(
      "<p><i>Noun</i></p><ol><li>A <b>thing</b>.</li></ol>",
    );
  });

  it("escapes the part of speech", () => {
    expect(definitionsHtml([{ partOfSpeech: '<img src=x onerror="alert(1)">', definitions: [] }])).toBe(
      "<p><i>&lt;img src=x onerror=&quot;alert(1)&quot;&gt;</i></p><ol></ol>",
    );
  });

  it("strips attributes and foreign tags from definitions", () => {
    const html = definitionsHtml([
      { partOfSpeech: "Verb", definitions: [{ definition: '<b onpointerenter="alert(1)">to</b> <a href="/wiki/x" onclick="x()">run</a>' }] },
    ]);
    expect(html).toBe("<p><i>Verb</i></p><ol><li><b>to</b> run</li></ol>");
  });

  it("limits to four entries of four definitions and survives missing fields", () => {
    const many = Array.from({ length: 6 }, (_, i) => ({
      partOfSpeech: `P${i}`,
      definitions: Array.from({ length: 6 }, (_, k) => ({ definition: `d${k}` })),
    }));
    const html = definitionsHtml(many);
    expect(html.match(/<p>/g)?.length).toBe(4);
    expect(html.match(/<li>/g)?.length).toBe(16);
    expect(definitionsHtml([{}, { definitions: [{}] }] as never)).toBe("<p><i></i></p><ol></ol><p><i></i></p><ol><li></li></ol>");
  });
});
