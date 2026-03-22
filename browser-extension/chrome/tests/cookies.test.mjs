import test from "node:test";
import assert from "node:assert/strict";

import { extractCookiesForPlatform } from "../src/cookies.js";

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
