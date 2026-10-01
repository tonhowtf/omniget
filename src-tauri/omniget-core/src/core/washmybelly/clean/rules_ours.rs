//! Rules of our own (GPL-3.0) for what the studies found missing or done
//! wrong upstream: developer caches on macOS and Linux with the right scope
//! (only `~/.cargo/registry/cache`, never the index or git checkouts; pnpm
//! through `pnpm store prune`), Docker through its own prune with a preview
//! of what it reclaims, and half-finished downloads that are days old.

use serde_json::Value;

use super::super::plan::Op;
use super::super::{Destination, Os, Risk};
use super::catalog::b;
use super::engine::{Env, Found};
use super::rule::Rule;

const ML: &[Os] = &[Os::Macos, Os::Linux];
const ALL: &[Os] = &[Os::Macos, Os::Linux, Os::Windows];

pub fn rules() -> Vec<Rule> {
    vec![
        b("ours.npm", ML, "dev", "npm cache")
            .t(&["~/.npm/_cacache", "~/.npm/_logs"])
            .running(&["npm"])
            .done(),
        b("ours.pnpm", ML, "dev", "pnpm store")
            .cmd("pnpm", &["store", "prune"], &["$PNPM_STORE"])
            .desc("Packages no project uses any more, removed by pnpm store prune.")
            .done(),
        b("ours.yarn", ML, "dev", "Yarn cache")
            .t(&["~/Library/Caches/Yarn", "$XDG_CACHE_HOME/yarn", "~/.yarn/berry/cache"])
            .running(&["yarn"])
            .done(),
        b("ours.pip", ML, "dev", "pip cache")
            .t(&["~/Library/Caches/pip", "$XDG_CACHE_HOME/pip"])
            .running(&["pip", "pip3"])
            .done(),
        b("ours.uv", ML, "dev", "uv cache")
            .t(&["~/Library/Caches/uv", "$XDG_CACHE_HOME/uv"])
            .running(&["uv"])
            .done(),
        b("ours.cargo", ML, "dev", "Cargo download cache")
            .t(&["~/.cargo/registry/cache"])
            .running(&["cargo"])
            .desc("Downloaded crate archives. The registry index and git checkouts stay.")
            .done(),
        b("ours.gradle", ML, "dev", "Gradle caches")
            .t(&["~/.gradle/caches"])
            .running(&["java", "gradle"])
            .desc("Gradle downloads dependencies again on the next build, which can take a while.")
            .done(),
        b("ours.cocoapods", &[Os::Macos], "dev", "CocoaPods cache")
            .t(&["~/Library/Caches/CocoaPods"])
            .running(&["pod"])
            .done(),
        b("ours.go_build", ML, "dev", "Go build cache")
            .t(&["~/Library/Caches/go-build", "$XDG_CACHE_HOME/go-build"])
            .running(&["go"])
            .done(),
        b("ours.composer", ML, "dev", "Composer cache")
            .t(&["~/Library/Caches/composer", "$XDG_CACHE_HOME/composer", "~/.composer/cache"])
            .done(),
        b("ours.bun", ML, "dev", "Bun cache")
            .t(&["~/.bun/install/cache"])
            .running(&["bun"])
            .done(),
        b("ours.deno", ML, "dev", "Deno cache")
            .t(&["~/Library/Caches/deno", "$XDG_CACHE_HOME/deno"])
            .running(&["deno"])
            .done(),
        b("ours.docker_buildx", ML, "dev", "Docker build cache")
            .t(&["~/.docker/buildx/cache"])
            .done(),
        b("ours.docker_prune", ML, "dev", "Docker: unused containers, networks and images")
            .cmd("docker", &["system", "prune", "-f"], &[])
            .desc("Stopped containers, unused networks, dangling images and build cache. Volumes and tagged images stay.")
            .done(),
        b("ours.incomplete_downloads", ALL, "user_files", "Unfinished downloads")
            .t(&["~/Downloads/*.crdownload", "~/Downloads/*.part", "~/Downloads/*.partial", "~/Downloads/*.download"])
            .delete()
            .age(3)
            .dest(Destination::Trash)
            .risk(Risk::Reversible)
            .desc("Downloads a browser left half done more than three days ago.")
            .done(),
    ]
}

/// `docker system df --format '{{json .}}'`: what `prune -f` would reclaim.
pub fn probe_docker(rule: &Rule, env: &Env) -> Vec<Found> {
    let Some(out) = env.output("docker", &["system", "df", "--format", "{{json .}}"]) else {
        return Vec::new();
    };
    let bytes = parse_docker_df(&out);
    if bytes == 0 {
        return Vec::new();
    }
    let super::rule::Action::Command { program, args, .. } = &rule.action else {
        return Vec::new();
    };
    vec![Found {
        path: "docker:system".into(),
        label: None,
        bytes,
        files: 0,
        op: Op::Command {
            program: program.clone(),
            args: args.clone(),
            measure: Vec::new(),
        },
        admin: false,
    }]
}

/// Sum of the "Reclaimable" column, except volumes (`prune -f` keeps them).
pub fn parse_docker_df(text: &str) -> u64 {
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok())
        .filter(|v| {
            v["Type"]
                .as_str()
                .is_some_and(|t| !t.to_ascii_lowercase().contains("volume"))
        })
        .filter_map(|v| {
            v["Reclaimable"]
                .as_str()
                .map(|s| s.split_whitespace().next().unwrap_or("").to_string())
        })
        .filter_map(|s| parse_decimal_size(&s))
        .sum()
}

/// Docker prints decimal units: `1.2GB`, `512.5MB`, `0B`, `3kB`.
pub fn parse_decimal_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: f64 = num.parse().ok()?;
    let mult = match unit.to_ascii_uppercase().as_str() {
        "" | "B" => 1.0,
        "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        _ => return None,
    };
    Some((n * mult) as u64)
}
