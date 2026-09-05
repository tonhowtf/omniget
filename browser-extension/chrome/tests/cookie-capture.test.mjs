import test from "node:test";
import assert from "node:assert/strict";

import { captureCookiesForTab, detectPlatformKind, rootDomainOf } from "../src/cookie-capture.js";

test("rootDomainOf keeps the registrable name under two-label public suffixes", () => {
  assert.equal(rootDomainOf("app.rocketseat.com.br"), "rocketseat.com.br");
  assert.equal(rootDomainOf(".rocketseat.com.br"), "rocketseat.com.br");
  assert.equal(rootDomainOf("www.bbc.co.uk"), "bbc.co.uk");
  assert.equal(rootDomainOf("music.youtube.com"), "youtube.com");
  assert.equal(rootDomainOf("hotmart.com"), "hotmart.com");
  assert.equal(rootDomainOf("localhost"), "localhost");
});

test("detectPlatformKind recognises course platforms", () => {
  assert.equal(detectPlatformKind("app.rocketseat.com.br"), "rocketseat");
  assert.equal(detectPlatformKind("consumer.hotmart.com"), "hotmart");
  assert.equal(detectPlatformKind("www.udemy.com"), "udemy");
  assert.equal(detectPlatformKind("music.youtube.com"), "youtube_music");
  assert.equal(detectPlatformKind("example.org"), "generic");
});

test("captureCookiesForTab asks for the registrable domain and ships the batch", async () => {
  const requested = [];
  const cookiesApi = {
    getAll(details, cb) {
      requested.push(details.domain);
      cb([
        { domain: ".rocketseat.com.br", name: "skylab_next_access_token_v4", value: "eyJ.fake", path: "/", secure: true, httpOnly: false, expirationDate: 1790784038.5 },
      ]);
    },
  };
  let sent = null;
  const send = async (cookies, meta) => { sent = { cookies, meta }; return { ok: true }; };
  const result = await captureCookiesForTab(
    { url: "https://app.rocketseat.com.br/classroom/kubernetes", title: "Kubernetes | Rocketseat" },
    { cookiesApi, send },
  );
  assert.deepEqual(requested, ["rocketseat.com.br"]);
  assert.equal(result.ok, true);
  assert.equal(result.domain, "rocketseat.com.br");
  assert.equal(result.platform_kind, "rocketseat");
  assert.equal(result.cookie_count, 1);
  assert.equal(sent.cookies[0].expires, 1790784038);
  assert.equal(sent.meta.alias, "Kubernetes | Rocketseat (rocketseat.com.br)");
});

test("captureCookiesForTab reports when the domain has no cookies", async () => {
  const cookiesApi = { getAll(_details, cb) { cb([]); } };
  const result = await captureCookiesForTab({ url: "https://app.rocketseat.com.br/" }, { cookiesApi, send: async () => ({ ok: true }) });
  assert.deepEqual(result, { ok: false, reason: "no-cookies-for-domain", domain: "rocketseat.com.br" });
});
