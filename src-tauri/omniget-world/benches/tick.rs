//! Headless bench of the simulation, in the mould of
//! `omniget-core/tests/bench_engine.rs`: ignored by default, run on demand,
//! prints raw numbers and nothing else.
//!
//! ```text
//! cargo test -p omniget-world --release --bench tick -- --ignored --nocapture
//! ```
//!
//! Without `--ignored` it prints one line and exits, so `cargo test` stays
//! fast. `harness = false` in `Cargo.toml` keeps it on stable Rust with no
//! criterion and no nightly `#[bench]`.
//!
//! Environment:
//! - `BENCH_AGENTS`  agent counts to measure, comma separated (default
//!   `1,8,24,64`)
//! - `BENCH_TICKS`   ticks per measurement (default `2000`)
//! - `BENCH_WARMUP`  ticks discarded first (default `200`)

use std::time::Instant;

use omniget_world::ents::EntId;
use omniget_world::map::{ChunkDef, MapDef, TileDef, CHUNK_AREA, CHUNK_TILES, EMPTY};
use omniget_world::sim::{hm, Decision, Routine, RoutineEntry};
use omniget_world::{AStar, Grid, Input, Interest, Map, Tile, World};

fn env_list(name: &str, default: &str) -> Vec<usize> {
    std::env::var(name)
        .unwrap_or_else(|_| default.to_string())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect()
}

fn env_num(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(default)
}

/// A square house of `chunks` x `chunks` chunks: open floor with a wall every
/// eight tiles and a gap in it, which is roughly the density of the real one
/// and keeps A* honest.
fn big_map(chunks: i32) -> MapDef {
    let palette = vec![
        TileDef {
            key: "floor/wood".into(),
            height: 0,
            occludes: false,
            footprint: [1, 1],
            walkable: true,
        },
        TileDef {
            key: "wall/plaster".into(),
            height: 32,
            occludes: true,
            footprint: [1, 1],
            walkable: false,
        },
        TileDef {
            key: "object/workbench".into(),
            height: 20,
            occludes: false,
            footprint: [2, 1],
            walkable: false,
        },
        TileDef {
            key: "object/bed".into(),
            height: 16,
            occludes: false,
            footprint: [2, 2],
            walkable: false,
        },
    ];
    let side = chunks * CHUNK_TILES as i32;
    let mut defs = Vec::new();
    for cy in 0..chunks {
        for cx in 0..chunks {
            let mut floor = vec![0u8; CHUNK_AREA];
            let mut wall = vec![EMPTY; CHUNK_AREA];
            let mut height = vec![0u8; CHUNK_AREA];
            for ty in 0..CHUNK_TILES {
                for tx in 0..CHUNK_TILES {
                    let gx = cx * CHUNK_TILES as i32 + tx as i32;
                    let gy = cy * CHUNK_TILES as i32 + ty as i32;
                    let edge = gx == 0 || gy == 0 || gx == side - 1 || gy == side - 1;
                    let partition = gx % 8 == 0 && gy % 8 != 3;
                    if edge || partition {
                        let i = ty * CHUNK_TILES + tx;
                        wall[i] = 1;
                        height[i] = 32;
                        floor[i] = 0;
                    }
                }
            }
            defs.push(ChunkDef {
                cx,
                cy,
                floor,
                wall,
                object: vec![],
                height,
                tint: Vec::new(),
            });
        }
    }
    MapDef {
        version: 1,
        id: "bench-house".into(),
        atlas: "world/tiles/casa-v1/atlas.json".into(),
        chunk_tiles: CHUNK_TILES as u32,
        palette,
        chunks: defs,
        slots: vec![],
        objects: vec![],
        rooms: vec![],
        markers: vec![],
    }
}

/// A world with `n` agents, each with a routine that keeps it walking.
fn populated(n: usize, chunks: i32) -> World {
    let side = chunks * CHUNK_TILES as i32;
    let mut w = World::new(0x0B00_B1E5, big_map(chunks)).expect("map");
    let mut inputs = Vec::new();
    for i in 0..n {
        let x = 1 + (i as i32 * 3) % (side - 2);
        let y = 1 + (i as i32 * 5) % (side - 2);
        inputs.push(Input::Spawn {
            ent: EntId(i as u32 + 1),
            name: format!("agent-{i}"),
            at: Tile::new(x, y),
        });
    }
    w.step(&inputs);
    let mut routines = Vec::new();
    for i in 0..n {
        let r = Routine::new(vec![
            RoutineEntry {
                minute: hm(0, (i % 60) as u16),
                decision: Decision::GoTo(Tile::new(side - 3, side - 3)),
            },
            RoutineEntry {
                minute: hm(0, ((i % 60) + 20) as u16),
                decision: Decision::GoTo(Tile::new(2, 2)),
            },
            RoutineEntry {
                minute: hm(0, ((i % 60) + 40) as u16),
                decision: Decision::Say("trabalhando".into()),
            },
        ]);
        routines.push(Input::SetRoutine {
            ent: EntId(i as u32 + 1),
            routine: r,
        });
    }
    w.step(&routines);
    w
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if !args
        .iter()
        .any(|a| a == "--ignored" || a == "--include-ignored")
    {
        println!("bench tick: ignored (pass -- --ignored --nocapture to run)");
        return;
    }
    let counts = env_list("BENCH_AGENTS", "1,8,24,64");
    let ticks = env_num("BENCH_TICKS", 2000);
    let warmup = env_num("BENCH_WARMUP", 200);

    println!("host: {} {}", std::env::consts::OS, std::env::consts::ARCH);
    println!("ticks per run: {ticks} (warmup {warmup})");
    println!();
    println!("| agents | ms/tick avg | ms/tick p95 | ms/tick max | diff bytes (10 Hz) | snapshot bytes |");
    println!("| ---: | ---: | ---: | ---: | ---: | ---: |");

    for &n in &counts {
        let mut w = populated(n, 4);
        for _ in 0..warmup {
            w.step(&[]);
        }
        let mut samples = Vec::with_capacity(ticks as usize);
        let mut diff_total = 0usize;
        let mut diff_count = 0usize;
        for _ in 0..ticks {
            let t0 = Instant::now();
            w.step(&[]);
            samples.push(t0.elapsed().as_secs_f64() * 1000.0);
            if let Ok(d) = w.diff_since(w.tick() - 1, &Interest::ALL) {
                diff_total += d.encode().len();
                diff_count += 1;
            }
        }
        samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
        let avg: f64 = samples.iter().sum::<f64>() / samples.len() as f64;
        let p95 = samples[(samples.len() * 95 / 100).min(samples.len() - 1)];
        let max = *samples.last().expect("samples");
        let snap = w.snapshot().encode().len();
        let diff_avg = diff_total.checked_div(diff_count).unwrap_or(0);
        println!("| {n} | {avg:.4} | {p95:.4} | {max:.4} | {diff_avg} | {snap} |");
    }

    // --- A* on a 64x64 map with walls ---
    println!();
    let map = Map::load(&big_map(4)).expect("map");
    let grid = Grid::from_map(&map);
    let mut astar = AStar::new();
    let mut out = Vec::new();
    let mut best = f64::MAX;
    let mut worst: f64 = 0.0;
    let mut total = 0.0;
    let runs = 500;
    for _ in 0..runs {
        let t0 = Instant::now();
        let ok = astar.find(&grid, Tile::new(1, 1), Tile::new(62, 62), &mut out);
        let us = t0.elapsed().as_secs_f64() * 1_000_000.0;
        assert!(ok, "the bench map must be connected");
        best = best.min(us);
        worst = worst.max(us);
        total += us;
    }
    println!(
        "astar 64x64 with walls: avg {:.1} us, min {:.1} us, max {:.1} us, {} steps, {} nodes",
        total / runs as f64,
        best,
        worst,
        out.len(),
        astar.last_expansions
    );

    // --- catch_up of eight hours ---
    for &n in &counts {
        let mut w = populated(n, 4);
        let t0 = Instant::now();
        w.catch_up(8 * 3_600_000);
        println!(
            "catch_up 8 h with {n} agents: {:.3} ms",
            t0.elapsed().as_secs_f64() * 1000.0
        );
    }

    // --- snapshot round trip ---
    let w = populated(24, 4);
    let snap = w.snapshot();
    let bytes = snap.encode();
    let t0 = Instant::now();
    for _ in 0..1000 {
        let _ = omniget_world::Snapshot::decode(&bytes).expect("decode");
    }
    println!(
        "snapshot decode, 24 agents, {} bytes: {:.1} us each",
        bytes.len(),
        t0.elapsed().as_secs_f64() * 1_000_000.0 / 1000.0
    );
}
