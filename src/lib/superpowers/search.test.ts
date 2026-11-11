import { describe, expect, it } from "vitest";
import { searchPowers } from "./search";

const music = { title: "Música", description: "Letras e rádio", keywords: "player playlist" };
const video = { title: "Vídeo", description: "Filmes com música de fundo", keywords: "jellyfin iptv" };
const read = { title: "Ler", description: "Livros e mangá", keywords: "ebook" };
const all = [video, read, music];

describe("searchPowers", () => {
  it("returns everything in the original order for a blank query", () => {
    expect(searchPowers(all, "   ")).toEqual(all);
  });

  it("ignores accents and case in both directions", () => {
    expect(searchPowers(all, "MANGA")).toEqual([read]);
    expect(searchPowers(all, "vídeo")).toEqual([video]);
    expect(searchPowers(all, "video")).toEqual([video]);
  });

  it("matches keywords that are not in the visible text", () => {
    expect(searchPowers(all, "jellyfin")).toEqual([video]);
  });

  it("requires every word, in any order", () => {
    expect(searchPowers(all, "radio letras")).toEqual([music]);
    expect(searchPowers(all, "radio ebook")).toEqual([]);
  });

  it("puts title matches before description matches", () => {
    expect(searchPowers(all, "musica")).toEqual([music, video]);
  });

  it("returns nothing when no item matches", () => {
    expect(searchPowers(all, "xyz")).toEqual([]);
  });

  it("matches the category label and the tags", () => {
    const tagged = [
      { ...music, category: "Mídia", tags: ["MCP"] },
      { ...video, category: "Mídia", tags: [] },
      { ...read, category: "Leitura", tags: ["Navegador"] },
    ];
    expect(searchPowers(tagged, "midia")).toEqual([tagged[0], tagged[1]]);
    expect(searchPowers(tagged, "mcp")).toEqual([tagged[0]]);
    expect(searchPowers(tagged, "navegador livros")).toEqual([tagged[2]]);
  });
});
