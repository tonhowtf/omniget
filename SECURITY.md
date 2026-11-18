# Security policy

## Supported versions

Security fixes go into the latest release only. Updates arrive inside the app, and every release is listed on [getomniget.com/download](https://getomniget.com/download) with the SHA-256 of each file.

## Reporting a vulnerability

Write to [security@getomniget.com](mailto:security@getomniget.com). Please do not open a public issue for a security problem.

Include what you can of the following:

- the OmniGet version and your operating system
- what an attacker can do, and what they need to get there
- steps or a proof of concept that reproduces it

You will get an answer saying the report arrived, then a second one once it has been reproduced or ruled out. A fix ships in a normal release, and the release notes credit you unless you ask to stay anonymous.

## Scope

In scope: the desktop app, the command-line tools and the MCP server in this repository, the browser extension, the updater, and the way OmniGet downloads and verifies the tools it manages (yt-dlp, FFmpeg, gallery-dl, aria2 and others).

Out of scope: vulnerabilities in those third-party tools themselves (report them upstream; tell us too if OmniGet makes them worse), sites OmniGet downloads from, and anything that needs an attacker who already controls your user account on the machine.

More on how releases are built and checked: [getomniget.com/security](https://getomniget.com/security).
