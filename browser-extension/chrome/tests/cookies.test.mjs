import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

import {
  COOKIE_DOMAINS_RESOURCE_PATH,
  DEFAULT_PLATFORM_COOKIE_DOMAINS,
  extractCookiesForPlatform,
  getPlatformCookieDomainsCacheSource,
  loadPlatformCookieDomains,
  resetPlatformCookieDomainsCacheForTesting,
} from "../src/cookies.js";

function mockGetAllCookies(cookieStore) {
  return async ({ domain }) => cookieStore[domain] ?? [];
}

const FAKE_COOKIES = {
  ".youtube.com": [
    {
      domain: ".youtube.com",
      httpOnly: true,
      path: "/",
      secure: true,
      expirationDate: 1735689600,
      name: "LOGIN_INFO",
      value: "fake-login-info",
    },
    {
      domain: ".youtube.com",
      httpOnly: false,
      path: "/",
      secure: true,
      expirationDate: 1735689600,
      name: "PREF",
      value: "tz=Europe.Paris",
    },
  ],
  ".google.com": [
    {
      domain: ".google.com",
      httpOnly: true,
      path: "/",
      secure: true,
      expirationDate: 1735689600,
      name: "SID",
      value: "fake-sid",
    },
  ],
};

test("extracts cookies for a known platform", async () => {
  const cookies = await extractCookiesForPlatform(
    "youtube",
    mockGetAllCookies(FAKE_COOKIES)
  );

  assert.ok(cookies);
  assert.equal(cookies.length, 3);
  assert.equal(cookies[0].name, "LOGIN_INFO");
  assert.equal(cookies[0].httpOnly, true);
  assert.equal(cookies[2].name, "SID");
  assert.equal(cookies[2].domain, ".google.com");
});

test("converts expirationDate to integer expires field", async () => {
  const cookies = await extractCookiesForPlatform(
    "youtube",
    mockGetAllCookies(FAKE_COOKIES)
  );

  for (const c of cookies) {
    assert.equal(typeof c.expires, "number");
    assert.equal(c.expires, Math.floor(c.expires));
  }
});

test("returns null for unknown platforms", async () => {
  const cookies = await extractCookiesForPlatform(
    "hotmart",
    mockGetAllCookies(FAKE_COOKIES)
  );

  assert.equal(cookies, null);
});

test("returns null when no cookies exist for the platform", async () => {
  const cookies = await extractCookiesForPlatform(
    "twitch",
    mockGetAllCookies({})
  );

  assert.equal(cookies, null);
});

test("handles session cookies with no expirationDate", async () => {
  const cookies = await extractCookiesForPlatform(
    "reddit",
    mockGetAllCookies({
      ".reddit.com": [
        {
          domain: ".reddit.com",
          httpOnly: false,
          path: "/",
          secure: false,
          name: "session_tracker",
          value: "abc",
        },
      ],
    })
  );

  assert.ok(cookies);
  assert.equal(cookies[0].expires, 0);
});

test("extracts cookies from multiple domains for twitter", async () => {
  const cookies = await extractCookiesForPlatform(
    "twitter",
    mockGetAllCookies({
      ".twitter.com": [
        { domain: ".twitter.com", httpOnly: false, path: "/", secure: true, expirationDate: 999, name: "ct0", value: "tok1" },
      ],
      ".x.com": [
        { domain: ".x.com", httpOnly: false, path: "/", secure: true, expirationDate: 999, name: "auth_token", value: "tok2" },
      ],
    })
  );

  assert.ok(cookies);
  assert.equal(cookies.length, 2);
  assert.equal(cookies[0].domain, ".twitter.com");
  assert.equal(cookies[1].domain, ".x.com");
});

test("uses default fallback when chrome.cookies is unavailable", async () => {
  const cookies = await extractCookiesForPlatform("youtube");
  assert.equal(cookies, null);
});

test("loadPlatformCookieDomains returns the default map when no chrome resource is available", async () => {
  resetPlatformCookieDomainsCacheForTesting();
  const domains = await loadPlatformCookieDomains();

  assert.deepEqual(domains, DEFAULT_PLATFORM_COOKIE_DOMAINS);
  assert.equal(getPlatformCookieDomainsCacheSource(), "default");
});

test("loadPlatformCookieDomains is cached after the first call", async () => {
  resetPlatformCookieDomainsCacheForTesting();
  const first = await loadPlatformCookieDomains();
  const second = await loadPlatformCookieDomains();

  assert.equal(first, second);
});

test("default domains include all currently detectable platforms", async () => {
  const required = [
    "youtube",
    "instagram",
    "tiktok",
    "twitter",
    "reddit",
    "twitch",
    "vimeo",
    "bilibili",
    "pinterest",
    "hotmart",
    "udemy",
    "bluesky",
    "telegram",
  ];
  for (const key of required) {
    assert.ok(
      Array.isArray(DEFAULT_PLATFORM_COOKIE_DOMAINS[key]) &&
        DEFAULT_PLATFORM_COOKIE_DOMAINS[key].length > 0,
      `DEFAULT_PLATFORM_COOKIE_DOMAINS missing platform ${key}`,
    );
  }
});

test("shipped cookies-domains.json matches DEFAULT_PLATFORM_COOKIE_DOMAINS", async () => {
  const jsonUrl = new URL(`../${COOKIE_DOMAINS_RESOURCE_PATH}`, import.meta.url);
  const json = JSON.parse(await readFile(jsonUrl, "utf8"));

  assert.deepEqual(
    json,
    Object.fromEntries(
      Object.entries(DEFAULT_PLATFORM_COOKIE_DOMAINS).map(([k, v]) => [k, [...v]]),
    ),
  );
});

test("extractCookiesForPlatform accepts a domainsOverride for injection testing", async () => {
  const cookies = await extractCookiesForPlatform(
    "mynewplatform",
    mockGetAllCookies({
      ".mynewplatform.com": [
        { domain: ".mynewplatform.com", httpOnly: false, path: "/", secure: true, expirationDate: 1, name: "n", value: "v" },
      ],
    }),
    { mynewplatform: [".mynewplatform.com"] },
  );

  assert.ok(cookies);
  assert.equal(cookies.length, 1);
  assert.equal(cookies[0].domain, ".mynewplatform.com");
});

test("bluesky platform is now data-driven and resolves to bsky domains", async () => {
  const cookies = await extractCookiesForPlatform(
    "bluesky",
    mockGetAllCookies({
      ".bsky.app": [
        { domain: ".bsky.app", httpOnly: true, path: "/", secure: true, expirationDate: 42, name: "auth", value: "x" },
      ],
    }),
  );

  assert.ok(cookies);
  assert.equal(cookies.length, 1);
  assert.equal(cookies[0].domain, ".bsky.app");
});

test("telegram platform is now data-driven and resolves to telegram.org", async () => {
  const cookies = await extractCookiesForPlatform(
    "telegram",
    mockGetAllCookies({
      ".telegram.org": [
        { domain: ".telegram.org", httpOnly: false, path: "/", secure: true, expirationDate: 0, name: "stel", value: "y" },
      ],
    }),
  );

  assert.ok(cookies);
  assert.equal(cookies.length, 1);
  assert.equal(cookies[0].domain, ".telegram.org");
});

test("rocketseat maps to the .com.br domain in defaults and resource", async () => {
  assert.deepEqual(DEFAULT_PLATFORM_COOKIE_DOMAINS.rocketseat, [".rocketseat.com.br"]);
  const raw = await readFile(new URL("../src/cookies-domains.json", import.meta.url), "utf8");
  assert.deepEqual(JSON.parse(raw).rocketseat, [".rocketseat.com.br"]);
});

test("extractCookiesForPlatform picks up the Rocketseat JWT cookie", async () => {
  const store = {
    ".rocketseat.com.br": [
      {
        domain: ".rocketseat.com.br",
        httpOnly: false,
        path: "/",
        secure: true,
        expirationDate: 1790784038,
        name: "skylab_next_access_token_v4",
        value: "eyJ.fake.jwt",
      },
    ],
  };
  const cookies = await extractCookiesForPlatform("rocketseat", mockGetAllCookies(store));
  assert.equal(cookies.length, 1);
  assert.equal(cookies[0].name, "skylab_next_access_token_v4");
  assert.equal(cookies[0].domain, ".rocketseat.com.br");
});
