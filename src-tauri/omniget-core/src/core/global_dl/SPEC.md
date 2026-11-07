# OmniGet connector spec

A connector tells OmniGet where the media on a site is. It is a folder:

```
my-connector/
  manifest.json
  connector.js
  GUIDE.md        how to use it, what needs a login, known limits
```

## manifest.json

```json
{
  "id": "example-videos",
  "name": "Example Videos",
  "version": "0.1.0",
  "author": "you",
  "license": "MIT",
  "domains": ["example.com", "cdn.example.net"],
  "match": ["https://www.example.com/watch/*", "https://www.example.com/series/**"],
  "login": { "url": "https://www.example.com/login" },
  "entry": "connector.js",
  "min_omniget": "1.0.0",
  "category": "courses",
  "permissions": ["cookies", "page_eval"],
  "options": [
    { "id": "subtitles", "label": "Subtitle languages", "type": "string", "default": "all" },
    { "id": "format", "label": "Subtitle format", "type": "select", "default": "vtt", "choices": ["vtt", "srt"] },
    { "id": "extras", "label": "Bonus material", "type": "boolean", "default": true }
  ]
}
```

- `domains`: the only hosts `ctx.fetch` may reach (subdomains included). Tree
  URLs on other hosts still download, but with no cookies and no credential
  headers: only `Referer`, `Origin`, `User-Agent`, `Accept`. Local addresses
  outside `domains` and opted-out sites are refused.
- `id`: 1-64 lowercase letters, digits, `-`, `_` or `.`, starting and ending with a letter or digit.
- `match`: URL patterns. `*` matches within one path segment, `**` matches across `/`, a
  pattern starting with `^` is a regex. Query and fragment are ignored unless the pattern has
  a `?`. A pattern never claims a URL whose host is outside `domains`. When several connectors
  match, a dev folder wins, then the longest pattern, then the lowest id.
- `login.url`: where "Log in and open" goes. Must be https and on one of the `domains`.
- `min_omniget`: older apps refuse the connector and show why.
- `permissions`: `cookies` for `ctx.cookies`, `page_eval` for `ctx.page.eval` and
  `ctx.page.evalAt`. Connectors installed from a package get only what they declare; dev
  folders get both.
- Hosts whose owners asked OmniGet not to support them are refused at load.
- A connector that cannot load is listed with the reason, not dropped.
- `options`: settings the person changes in the app (`boolean`, `string`,
  `select` with `choices`). The script reads them in `ctx.options`.

## connector.js

Plain JavaScript (QuickJS, ES2020). Define `resolve(url, ctx)`; it may be
`async`. No `require`, no file system, no sockets.

```js
function resolve(url, ctx) {
  const id = url.match(/series\/(\d+)/)[1];
  const data = ctx.fetchJson(`https://www.example.com/api/series/${id}`);
  return {
    title: data.title,
    children: data.seasons.map((s) => ({
      title: s.name,
      children: s.episodes.map((e) => ({
        title: e.title,
        kind: "video",
        drm: !!e.license,
        source: { format: "hls", url: e.playlist, headers: { Referer: "https://www.example.com/" } },
      })),
    })),
  };
}
```

### ctx

- `ctx.url`, `ctx.tabId`: the page being resolved and its browser tab.
- `ctx.options`: the option values (defaults, then what the person saved).
- `ctx.fetch(url, { method, headers, body })` → `{ status, ok, url, headers, text(), json() }`.
  Synchronous. When a tab of the same origin is open, the request runs inside
  it: the site's cookies, TLS and bot checks are the browser's own. Otherwise
  it goes out from OmniGet with the browser's cookies and user agent.
- `ctx.fetchJson(url, init)`: `fetch` + status check + `json()`.
- `ctx.cookies(domain)` → `{ name: value }` from the browser, for a declared domain.
- `ctx.page.eval(js)` → value of an expression evaluated in the page's tab
  (promises are awaited). Use it to read state the page already has.
- `ctx.page.evalAt(url, js, { keep })` → the same in a tab of `url`'s origin, for state kept
  by another origin of a declared domain. The tab is opened when none exists and closed after
  the call unless `keep` is true.
- `ctx.sniffed()` → media the tab already loaded:
  `[{ url, kind, mime, headers, page_url, page_title }]`.
- `ctx.log(...)`: shows up in the dry-run output.
- `ctx.query({ a: 1 })` → `"a=1"`.

## The tree

A node is a folder (`children`) or an item (`source`).

- `title`: shown and used for the file name.
- `kind`: `folder | video | audio | file | subtitle | text | link`.
- `drm`: `true` when the media needs a license (Widevine, FairPlay,
  PlayReady). OmniGet never downloads it and reports it as skipped.
- `note`: a short line shown next to the item.
- `source.format`:
  - `direct`: one file (`url`, optional `headers`).
  - `hls`: an `.m3u8` playlist; the best rendition is merged into one file.
  - `dash`: an `.mpd` manifest; handed to the Downloads queue (yt-dlp).
  - `ytdlp`: any URL yt-dlp understands; handed to the Downloads queue.
  - `text`: `content` is written as is (an article in HTML or Markdown).
- `source.ext`: extension when the URL does not show it.
- `source.audio`: URL of a separate audio track; it is joined with the video
  into one file (needs FFmpeg).
- `source.filename`: exact file name; otherwise `NN - title.ext`, numbered
  within its folder. Folders are named `NN - title`. A name without an
  extension gets the one from `ext` or the URL. Items with the same name in one
  folder are numbered: `Notes.pdf`, `Notes (2).pdf`.
- `source.convert`: `"srt"` turns a downloaded WebVTT file into SubRip.
- `key`: a stable id your connector understands (`"episode:123"`).

## Lazy items and expiring links

Signed media links often expire within hours, and a long queue outlives
them. Give such items a `key` and `source: { format: "lazy", filename }`
instead of the link, and define `resolveItem`:

```js
function resolveItem(key, ctx) {
  const id = key.split(":")[1];
  const e = ctx.fetchJson(`https://www.example.com/api/episodes/${id}`);
  if (e.license) return { drm: true };
  return { format: "hls", url: e.playlist };
}
```

OmniGet calls it right before the item downloads (`ctx.url` is the page the
tree came from). When a download fails with 401, 403 or 410, OmniGet reloads a
tab of that site (so its bot check is fresh), calls `resolveItem` again and
retries once. `resolveItem` returns a source, or a node (`{ drm, source }`).
A `filename` or `ext` it returns replaces what the tree said.
Resolving the tree also gets faster: fetch per-item details here, not in
`resolve`.

While a job runs, OmniGet keeps a tab of the page's site open in the
browser, and downloads go out with the browser's cookies and user agent for
their host.

Files that already exist with content are skipped, so a download can be run
again to fetch only what is missing.

## Workflow for writing one

1. `gd_browser_open` with the site (`window: true` for a visible window), log in by hand if needed.
2. Browse to a page with media, then `gd_network_log` (filter by domain or
   `types: ["XHR","Fetch"]`) to find the API calls; `gd_response_body` shows
   one response.
3. `gd_connector_scaffold`, write `resolve`, then `gd_connector_run` on a page
   URL until the tree is right. It downloads nothing. `gd_connector_run` with
   `item_key` runs `resolveItem` for one item; `options` overrides settings.
4. `gd_connector_install_dev` links the folder so the Global Downloader uses it.

## Packages

A `.omniconnector` file is a zip with `manifest.json`, the entry script and `GUIDE.md`.
`node scripts/connectors-build.mjs <dir>` builds the packages and the signed `catalog.json`.
