# Accessibility

OmniGet should be usable with a keyboard, with a screen reader, with less motion and in your own language. This page says what the app does today, where it still falls short, and how to report a barrier. It describes what is true now, and it changes in the same pull request as the code.

## What the app does today

- **Less motion.** Settings → Appearance → Reduce motion swaps movement for simple fades. If your system already asks for less motion, the app follows it without the switch.
- **Less transparency.** Reduce transparency, in the same place, uses solid backgrounds instead of see-through panels.
- **Themes.** Light and dark themes, plus more under More themes.
- **Languages.** The interface is translated into 12 languages on [Weblate](https://hosted.weblate.org/engage/omniget/), and anyone can add or fix one in the browser.
- **Labels and roles.** Buttons, dialogs and lists carry names and roles for assistive technology. The accessibility lint of `svelte-check` runs on every pull request and has to pass.
- **No mouse needed to drive it.** Everything the app does is also reachable through the local MCP server and `omniget-cli`, so an agent or a script can run a task that the interface makes hard.

## Known barriers

- **No outside audit.** OmniGet has not been through a formal accessibility audit, and it does not claim conformance with WCAG.
- **Screen readers.** Nobody who uses VoiceOver, NVDA or Orca every day has gone through the app from start to finish yet. Reports are collected in [#448](https://github.com/tonhowtf/omniget/issues/448).
- **Keyboard gaps.** The source has several dozen places where an accessibility lint is switched off (`svelte-ignore a11y_`). Some are fine, such as a backdrop that closes a dialog which Escape also closes. Others are click targets that still need a keyboard path; the list is in [#446](https://github.com/tonhowtf/omniget/issues/446).
- **The Canvas.** It is a spatial surface built around a pointer. Arranging and connecting items with the keyboard alone is limited.
- **Right to left.** Persian is translated, and the layout does not mirror yet ([#447](https://github.com/tonhowtf/omniget/issues/447)).
- **Captions.** The built-in players show the subtitles a download came with. They do not create captions for media that has none.

## Report a barrier

- [Open an issue](https://github.com/tonhowtf/omniget/issues/new/choose) with the bug form. Say which screen, what you were trying to do, and the assistive technology and system you use.
- No GitHub account: [getomniget.com/contact](https://getomniget.com/contact).

A barrier that stops you from using a feature is a bug, and it is handled like one.

## For contributors

- Keep `pnpm check` clean. Do not add a `svelte-ignore a11y_` comment to silence a warning on a control a person has to reach; give it a keyboard path and a name instead.
- New motion respects Reduce motion and the system setting.
- Interface text is born in `src/lib/i18n/en.json`, never hard-coded, so it can be translated and read aloud in the user's language.
- Removing one of the suppressions in [#446](https://github.com/tonhowtf/omniget/issues/446) is a good first pull request.
