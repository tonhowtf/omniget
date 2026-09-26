# Notice to platform owners

If you own or represent a platform and want OmniGet to stop supporting it, this page explains how to ask and what happens next.

## How to ask

Email **tonhowtf@gmail.com** from an address at your company's domain. Include:

- the platform name;
- the domains it uses (for example `example.com` and `app.example.com`);
- a contact for the confirmation.

No legal notice is needed. A plain request from the owner is enough.

## What happens

1. The platform comes out of the README, its translations, `llms.txt` and every other document this project publishes.
2. Any code this project maintains that is specific to the platform is removed, in the app and in the official add-ons.
3. Its domains go on the opt-out list in `src-tauri/omniget-core/src/core/platform_optout.rs`. From then on:
   - a link to the platform pasted in the app, sent by the browser extension, by the hotkey or by the MCP server is refused with the message "The owner of this site asked OmniGet not to support it.";
   - no connector that declares one of these domains is loaded, whoever wrote it.
4. You get a written confirmation with the version and the commits that made the change.

The list covers each domain and all of its subdomains.

## Current opt-out list

| Domains | Added |
|---|---|
| `kiwify.com.br`, `kiwify.com`, `kiwify.app` | 2026-09-24 |
