# Governance

How decisions are made in OmniGet today, and what happens to the project if the maintainer is not around. This page describes what is true now. When it changes, this file changes in the same pull request.

## Who decides

OmniGet has one maintainer: Antonio Leandro ([tonhowtf](https://github.com/tonhowtf)). He reviews and merges every pull request, decides what goes into a release, approves every signing request and publishes the files.

Everyone else takes part through pull requests, issues, translations and the community chat. The contributors are listed on the [contributors page](https://github.com/tonhowtf/omniget/graphs/contributors).

## How a change gets in

1. It arrives as a pull request. Larger changes are easier to accept when an issue or a discussion came first.
2. CI has to pass, and the pull request has to follow [CONTRIBUTING.md](CONTRIBUTING.md).
3. The maintainer reviews it, and may ask for changes or decline it.
4. Merged work ships in the next release and is listed in [CHANGELOG.md](CHANGELOG.md).

Product direction follows one rule that settles most arguments: the MCP server is the main product and the downloader comes second.

## Where things are decided in public

- Bugs and feature requests: [Issues](https://github.com/tonhowtf/omniget/issues).
- Questions and ideas: [Discussions](https://github.com/tonhowtf/omniget/discussions) and the [community chat](https://discord.gg/jgdxyPy7Vn).
- What is being worked on: the public board at [getomniget.com/contact](https://getomniget.com/contact).
- Security reports: privately, as [SECURITY.md](SECURITY.md) says.
- Requests from platform owners: [PLATFORM-OWNERS.md](PLATFORM-OWNERS.md).

## One maintainer is a risk

This project depends on one person, and this page does not hide it. There is no named successor today.

What does not depend on him:

- **The code.** OmniGet is GPL-3.0. The full source, the build instructions and the release workflow are in this repository, and anyone may fork it and ship their own builds.
- **The app you installed.** It runs on your computer and needs no account. Downloads, the MCP server and the tools keep working without any server of this project.

What does depend on him:

- **Official releases and the updater.** The updater key and the code-signing approvals are held by the maintainer. A fork has to sign with its own key, and an installed OmniGet will not update itself to a fork.
- **getomniget.com**, which hosts the downloads, the update manifest and the docs.
- **The relay used to send files between devices.** It can be pointed somewhere else with `OMNIGET_RELAY`.

## Forks

Forks are welcome and legal. To keep users from installing something they did not mean to, [getomniget.com/forks](https://getomniget.com/forks) says which project is the original and which names belong to forks.

## Changing this page

Open a pull request against this file. The maintainer decides.
