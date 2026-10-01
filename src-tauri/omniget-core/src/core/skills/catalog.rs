//! The pinned `OpenRouterTeam/skills` catalog.
//!
//! Verified on 18/09/2026 with `gh api repos/OpenRouterTeam/skills` and a
//! download of the tarball:
//!
//! - repository: <https://github.com/OpenRouterTeam/skills> (256 stars, no
//!   `LICENSE` file in the tree and `license: null` in the API — the skills
//!   carry no declared licence, which the UI must say before installing);
//! - commit of `main` at verification: `012c823da319ef10ee899a64aacd10e83ce39c74`
//!   (2026-08-25);
//! - tarball: <https://codeload.github.com/OpenRouterTeam/skills/tar.gz/012c823da319ef10ee899a64aacd10e83ce39c74>,
//!   503 928 bytes, sha256 `26cb531d...` ([`TARBALL_SHA256`]).
//!   The `refs/heads/main` tarball is deliberately *not* pinned: it moves.
//!
//! The plan says "the 16 skills of OpenRouterTeam". At this commit the repo has
//! **17** under `skills/`, all with a `SKILL.md` whose `name` matches its
//! folder. They are listed below with the sha256 of each `SKILL.md`, so an
//! install can be checked against what was audited ([`verify_installed`]).
//!
//! Nothing here touches the network on its own: [`install`] shallow-clones with
//! the system `git` only when the user asks for one entry, and the UI confirms
//! first.

use std::path::Path;

use super::install;
use super::SkillError;

/// Clone URL of the catalog repository.
pub const REPO_URL: &str = "https://github.com/OpenRouterTeam/skills.git";
/// Web page of the repository, for the UI.
pub const REPO_HTML_URL: &str = "https://github.com/OpenRouterTeam/skills";
/// Commit audited on 18/09/2026; installs pin to it.
pub const COMMIT: &str = "012c823da319ef10ee899a64aacd10e83ce39c74";
/// Date the commit above was authored.
pub const COMMIT_DATE: &str = "2026-08-25T17:16:29Z";
/// Immutable tarball of [`COMMIT`].
pub const TARBALL_URL: &str =
    "https://codeload.github.com/OpenRouterTeam/skills/tar.gz/012c823da319ef10ee899a64aacd10e83ce39c74";
/// sha256 of the tarball at [`TARBALL_URL`] (503 928 bytes).
pub const TARBALL_SHA256: &str = "26cb531d298028fc9a6c13a514fd518e467ca2e5dd0ba1c3e5e4e3c8f834e323";
/// No licence is declared anywhere in the repository.
pub const LICENSE: Option<&str> = None;
/// When the facts above were checked, by hand, against the live repository.
pub const VERIFIED_AT: &str = "2026-09-18";

/// One installable entry of the catalog. All data is baked in, so the list
/// renders with no network call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct CatalogEntry {
    pub name: &'static str,
    pub description: &'static str,
    /// Path of the skill inside the repository.
    pub subdir: &'static str,
    /// sha256 of the entry's `SKILL.md` at [`COMMIT`].
    pub skill_md_sha256: &'static str,
    /// Files the skill carries, at [`COMMIT`].
    pub files: u32,
    /// Bytes the skill carries, at [`COMMIT`].
    pub bytes: u64,
}

impl CatalogEntry {
    /// Web page of this skill, for the "view source" link.
    pub fn html_url(&self) -> String {
        format!("{REPO_HTML_URL}/tree/{COMMIT}/{}", self.subdir)
    }
}

/// The catalog, in the order the repository lists it.
pub const ENTRIES: &[CatalogEntry] = &[
    CatalogEntry {
        name: "create-agent-tui",
        description: "Scaffolds a complete agent TUI in TypeScript using @openrouter/agent — like create-react-app for terminal agents. Generates a customizable terminal interface with three input styles, four tool display modes, ASCII banners, streaming output, session persistence, and configurable tools. Use when building an agent, creating a TUI, scaffolding an agent project, or building a coding assistant.",
        subdir: "skills/create-agent-tui",
        skill_md_sha256: "73b1c691b4bd5797c2fdc1f3468e2220983eb842863fb6b3d22a6e59f361b8a3",
        files: 46,
        bytes: 503802,
    },
    CatalogEntry {
        name: "create-headless-agent",
        description: "Scaffolds a headless agent in TypeScript using @openrouter/agent and Bun — for CLI tools, API servers, queue workers, and pipelines. No terminal UI. Use when building a headless agent, programmatic agent, CLI tool that uses AI, batch agent, pipeline agent, API agent, agent without a UI, or agent service.",
        subdir: "skills/create-headless-agent",
        skill_md_sha256: "5d7c6bd9763fac5a6cd647cabb04f5c653dba870db4b0d1edbf8f2c4ff8e4da0",
        files: 28,
        bytes: 137883,
    },
    CatalogEntry {
        name: "install-ori-harness",
        description: "Install Ori and run the user's existing coding agent CLI through Ori on OpenRouter, with OAuth sign-in, model selection, upgrades, and install verification.",
        subdir: "skills/install-ori-harness",
        skill_md_sha256: "9f83871610998a6cc983036f9fe9b87dc086e9b03374923c48ecab871006160e",
        files: 2,
        bytes: 4672,
    },
    CatalogEntry {
        name: "openrouter-agent-migration",
        description: "Migration guide from @openrouter/sdk to @openrouter/agent for callModel, tool(), stop conditions, and agent features. This skill should be used when code imports callModel, tool(), or stop conditions from @openrouter/sdk and needs to migrate to @openrouter/agent.",
        subdir: "skills/openrouter-agent-migration",
        skill_md_sha256: "755575267297f071c6f2849dc7c4365281f1abbbc4cf941cc2a7c14f03d81f23",
        files: 3,
        bytes: 13661,
    },
    CatalogEntry {
        name: "openrouter-analytics",
        description: "Answer natural-language questions about a user's OpenRouter usage data — spend, request volume, model breakdown, latency, token usage, and cost optimization. Use when the user asks about their API usage, billing, costs, top models, traffic patterns, or wants to optimize their OpenRouter spend.",
        subdir: "skills/openrouter-analytics",
        skill_md_sha256: "1af096b64105a3ca71c6ff8a7d857dd4577f27dc3f6f3e713768daa2c990599f",
        files: 8,
        bytes: 40906,
    },
    CatalogEntry {
        name: "openrouter-analytics-query",
        description: "Construct and execute analytics queries against the OpenRouter API — full parameter reference for metrics, dimensions, filters, time ranges, ordering, and pagination. Use when building or debugging an analytics query, understanding the request/response shape, or handling query errors.",
        subdir: "skills/openrouter-analytics-query",
        skill_md_sha256: "e32c5c64db354b174cdd45e50f70692c48c1970f3ebd9a9e1ae7744803020ce3",
        files: 2,
        bytes: 18207,
    },
    CatalogEntry {
        name: "openrouter-analytics-schema",
        description: "Discover the OpenRouter analytics schema — available metrics, dimensions, filter operators, and granularities. Use when you need to know what analytics data is queryable, what dimensions you can break down by, or how to map a user's question to the right metric/dimension combination.",
        subdir: "skills/openrouter-analytics-schema",
        skill_md_sha256: "be9e5e2a3873bbea05228333dc3ec8ddc91ff7ed8400913a5f4388ffc57d2b44",
        files: 2,
        bytes: 15005,
    },
    CatalogEntry {
        name: "openrouter-benchmarks",
        description: "Query OpenRouter's Benchmarks API for model benchmark rankings and scores. Use when the user asks for benchmark-backed model selection, model rankings by coding/intelligence/agentic ability, Artificial Analysis or Design Arena ELO/win-rate results, benchmark citations, or wants to call GET /api/v1/benchmarks. Also use alongside openrouter-models when the user asks what model should power an app, product, workflow, or use case and benchmark evidence could inform or rule out part of the recommendation, including creative writing, editing, coding, design, agentic, or intelligence-heavy apps. Do not use for OpenRouter usage analytics, billing/spend analysis, generation metadata, provider uptime/latency, generic model pricing/capability lookup without any selection or benchmark-relevance decision, or creating an evaluation suite for a local app.",
        subdir: "skills/openrouter-benchmarks",
        skill_md_sha256: "6bfc172fe75e95c9b2f9df0510d61f5913663e123f70ae4d94e351a9401940fd",
        files: 4,
        bytes: 12840,
    },
    CatalogEntry {
        name: "openrouter-generations",
        description: "Retrieve detailed metadata and stored content for individual OpenRouter generations. Use when the user wants to inspect a specific request — its cost, latency, token usage, provider routing, or the actual prompt/completion text — or is debugging a failed or unexpected generation.",
        subdir: "skills/openrouter-generations",
        skill_md_sha256: "2976e9127f06b711727ae64d926bc40321f92633d090b4682f652fa5a379c147",
        files: 7,
        bytes: 38649,
    },
    CatalogEntry {
        name: "openrouter-images",
        description: "Generate images from text prompts and edit existing images using OpenRouter's dedicated Image API. Use when the user asks to create, generate, or make an image, picture, or illustration from a description, or wants to edit, modify, transform, or alter an existing image with a text prompt.",
        subdir: "skills/openrouter-images",
        skill_md_sha256: "cc1964ca7a944bd5b560428544b839e79e437167e460113a8a0121ac449e907d",
        files: 8,
        bytes: 40941,
    },
    CatalogEntry {
        name: "openrouter-models",
        description: "Query OpenRouter for available AI models, pricing, capabilities, throughput, and provider performance. Use when the user asks about available OpenRouter models, model pricing, model context lengths, model capabilities, provider latency or uptime, throughput limits, supported parameters, wants to search/filter/compare models, or find the fastest provider for a model.",
        subdir: "skills/openrouter-models",
        skill_md_sha256: "e3d0b75557c7d021326e17e7b8bbfb3dc410b0024e117bb0d531a8c024023ece",
        files: 10,
        bytes: 45607,
    },
    CatalogEntry {
        name: "openrouter-oauth",
        description: "Implement \"Sign In with OpenRouter\" using OAuth PKCE — framework-agnostic, no SDK or client registration required. Use when the user wants to add OpenRouter login, authentication, sign-in buttons, OAuth, or AI model inference API keys for browser-based apps. No client registration, no backend, no secrets required.",
        subdir: "skills/openrouter-oauth",
        skill_md_sha256: "984a7340cb83bacde0da41c5cf7362260b57540df82e9490da07283b538e86c8",
        files: 2,
        bytes: 10654,
    },
    CatalogEntry {
        name: "openrouter-stt",
        description: "Transcribe speech to text using OpenRouter's speech-to-text API. Use when the user asks to transcribe audio, convert speech to text, extract a transcript from a recording or meeting, caption a video's audio, or mentions STT, speech-to-text, ASR, or transcription.",
        subdir: "skills/openrouter-stt",
        skill_md_sha256: "753842c8f017ee01e4afde843b55f1499cefc9a0f81aeaca4e68925726777fd4",
        files: 2,
        bytes: 9377,
    },
    CatalogEntry {
        name: "openrouter-tts",
        description: "Generate speech audio from text using OpenRouter's text-to-speech API. Use when the user asks to synthesize speech, narrate text, create a voiceover, generate an audiobook clip, read text aloud, convert text to an audio file, or mentions TTS, text-to-speech, or voice synthesis.",
        subdir: "skills/openrouter-tts",
        skill_md_sha256: "ddfdc0be19d7684c76482540f349f2dff6d04566c4cdf158619af48387208927",
        files: 2,
        bytes: 10653,
    },
    CatalogEntry {
        name: "openrouter-typescript-sdk",
        description: "Complete reference for integrating with 300+ AI models through the OpenRouter TypeScript SDK and Agent packages using the callModel pattern",
        subdir: "skills/openrouter-typescript-sdk",
        skill_md_sha256: "a37f891099855312ad941fc722ad2ccdf515c0bf4fdf773cd8b13a520f68b1d7",
        files: 3,
        bytes: 34228,
    },
    CatalogEntry {
        name: "openrouter-video",
        description: "Generate videos from text prompts (and optional reference or frame images) using OpenRouter's asynchronous video generation API. Use when the user asks to create, generate, or make a video or animation from a description, animate an existing image, or turn a prompt into a short video clip.",
        subdir: "skills/openrouter-video",
        skill_md_sha256: "5acb20a0eb0eadac5bdf7de46854631d4db872eef1452aad791b864b086663a8",
        files: 2,
        bytes: 7227,
    },
    CatalogEntry {
        name: "spawn-ori-eval",
        description: "Spawn Ori as a subprocess to run a throwaway model eval on a pinned harness and model, then relay the results. Use when the user asks which model they should use, wants to compare models, wants to measure whether their agent or prompt does the right thing, wants to catch regressions in agent behavior, or asks how good their current model is. Applies to any codebase in any language. Do not use for plain unit tests that involve no model, and do not use to re-run an eval that already exists (run `ori eval <file>` directly).",
        subdir: "skills/spawn-ori-eval",
        skill_md_sha256: "24ee54ab75b1c62d0306d75d0fc9196d4095013fc33015495e85ba3d307d7a29",
        files: 2,
        bytes: 32898,
    },
];

/// The whole catalog.
pub fn entries() -> &'static [CatalogEntry] {
    ENTRIES
}

/// One entry by name.
pub fn entry(name: &str) -> Option<&'static CatalogEntry> {
    ENTRIES.iter().find(|e| e.name == name)
}

/// Install one catalog entry into the default skills directory. Shallow clone
/// of [`REPO_URL`] pinned at [`COMMIT`], then the entry's subdirectory. The UI
/// confirms with the user before calling this: it runs `git` and hits the
/// network.
pub fn install(name: &str) -> Result<install::InstallOutcome, SkillError> {
    install_in(&install::skills_dir()?, name)
}

/// [`install`] against an explicit root. Like every other install it comes back
/// parked when the scan is over the threshold — a pinned commit is no promise
/// of safety, only of reproducibility.
pub fn install_in(root: &Path, name: &str) -> Result<install::InstallOutcome, SkillError> {
    let entry = entry(name).ok_or_else(|| {
        SkillError::new(
            super::ERR_SKILL_NOT_FOUND,
            format!("`{name}` is not in the OpenRouter catalog"),
        )
    })?;
    install::install_from_git_in(root, REPO_URL, Some(COMMIT), Some(entry.subdir))
}

/// sha256 of a byte slice, lowercase hex.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// True when the installed `SKILL.md` is byte-for-byte the audited one. A
/// mismatch is not an error by itself — the skill may simply be newer — but the
/// UI can show "changed since audit".
pub fn verify_installed(root: &Path, name: &str) -> Result<bool, SkillError> {
    let Some(entry) = entry(name) else {
        return Ok(false);
    };
    let file = install::skill_path(root, name)?.join(super::manifest::SKILL_FILE);
    let bytes = std::fs::read(&file)
        .map_err(|e| SkillError::io(&format!("reading {}", file.display()), &e))?;
    Ok(sha256_hex(&bytes) == entry.skill_md_sha256)
}
