//! Benchmark do motor de download contra um servidor local (sem o limite do
//! link). Ignorado por padrão; roda com:
//!
//! ```text
//! BENCH_BASE=http://127.0.0.1:8765 BENCH_FILE=big1g.bin BENCH_MODE=raw \
//!   cargo test -p omniget-core --features desktop --release --test bench_engine -- --ignored --nocapture
//! ```
//!
//! Variáveis: `BENCH_SEGMENTS` (lista, ex. `1,2,4,8,16`), `BENCH_SEGSIZE_MB`
//! (lista), `BENCH_PARALLEL` (downloads simultâneos no cenário de stress),
//! `BENCH_YTDLP` (caminho do yt-dlp para o cenário generic), `BENCH_SMALL_N`.

use std::path::PathBuf;
use std::time::Instant;

use omniget_core::core::http_fetcher::{HttpFetcher, HttpFetcherConfig};
use omniget_core::models::progress::ProgressUpdate;

fn env_list(name: &str, default: &str) -> Vec<u64> {
    std::env::var(name)
        .unwrap_or_else(|_| default.to_string())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect()
}

fn out_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("omniget-bench-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn drain() -> tokio::sync::mpsc::Sender<ProgressUpdate> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ProgressUpdate>(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    tx
}

fn mib_s(bytes: u64, secs: f64) -> f64 {
    bytes as f64 / 1048576.0 / secs.max(1e-6)
}

async fn run_fetcher(
    client: &reqwest::Client,
    url: &str,
    out: PathBuf,
    segments: usize,
    seg_mb: u64,
) -> (u64, f64) {
    let cfg = HttpFetcherConfig {
        concurrent_segments: segments,
        segment_size_hint: seg_mb * 1024 * 1024,
        min_size_for_chunked: if segments > 1 { 1 } else { u64::MAX },
        use_sidecar_resume: false,
        steal_threshold: if std::env::var("BENCH_NO_STEAL").is_ok() {
            std::time::Duration::from_secs(3600)
        } else {
            HttpFetcherConfig::default().steal_threshold
        },
        ..Default::default()
    };
    let _ = std::fs::remove_file(&out);
    let t = Instant::now();
    let res = HttpFetcher::new(client.clone(), url.to_string(), out.clone())
        .with_config(cfg)
        .download(drain())
        .await
        .expect("download");
    let secs = t.elapsed().as_secs_f64();
    let _ = std::fs::remove_file(&out);
    (res.bytes_written, secs)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore]
async fn bench_engine() {
    let Ok(base) = std::env::var("BENCH_BASE") else {
        return;
    };
    let file = std::env::var("BENCH_FILE").unwrap_or_else(|_| "mid100m.bin".into());
    let mode = std::env::var("BENCH_MODE").unwrap_or_else(|_| "raw".into());
    let url = format!("{base}/{file}");
    let dir = out_dir();
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(64)
        .build()
        .unwrap();

    println!("\n## modo `{mode}` · arquivo `{file}`\n");
    println!("| cenário | config | tempo | MiB/s |");
    println!("|---|---|---|---|");

    if mode == "small" {
        let n: u64 = std::env::var("BENCH_SMALL_N").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
        // sequencial
        let t = Instant::now();
        let mut total = 0u64;
        for i in 1..=n {
            let u = format!("{base}/small_{i}.bin");
            let (b, _) = run_fetcher(&client, &u, dir.join(format!("s{i}.bin")), 1, 4).await;
            total += b;
        }
        let secs = t.elapsed().as_secs_f64();
        println!("| {n} arquivos de 1 MiB, sequencial | 1 conexão | {secs:.2}s | {:.1} ({:.0} ms/arquivo) |", mib_s(total, secs), secs * 1000.0 / n as f64);
        for par in env_list("BENCH_PARALLEL", "4,8") {
            let t = Instant::now();
            let mut handles = Vec::new();
            let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(par as usize));
            for i in 1..=n {
                let c = client.clone();
                let u = format!("{base}/small_{i}.bin");
                let o = dir.join(format!("p{i}.bin"));
                let sem = sem.clone();
                handles.push(tokio::spawn(async move {
                    let _p = sem.acquire().await.unwrap();
                    run_fetcher(&c, &u, o, 1, 4).await.0
                }));
            }
            let mut total = 0u64;
            for h in handles {
                total += h.await.unwrap();
            }
            let secs = t.elapsed().as_secs_f64();
            println!("| {n} arquivos de 1 MiB, {par} em paralelo | fila | {secs:.2}s | {:.1} ({:.0} ms/arquivo) |", mib_s(total, secs), secs * 1000.0 / n as f64);
        }
        return;
    }

    if mode == "stress" {
        for par in env_list("BENCH_PARALLEL", "2,4,8") {
            let segs: usize = std::env::var("BENCH_STRESS_SEGMENTS").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
            let t = Instant::now();
            let mut handles = Vec::new();
            for i in 0..par {
                let c = client.clone();
                let u = url.clone();
                let o = dir.join(format!("stress{i}.bin"));
                handles.push(tokio::spawn(async move { run_fetcher(&c, &u, o, segs, 4).await.0 }));
            }
            let mut total = 0u64;
            for h in handles {
                total += h.await.unwrap();
            }
            let secs = t.elapsed().as_secs_f64();
            println!("| {par} downloads simultâneos do mesmo arquivo | {segs} segmentos cada | {secs:.2}s | {:.1} agregado |", mib_s(total, secs));
        }
        return;
    }

    if mode == "ytdlp" {
        let ytdlp = PathBuf::from(std::env::var("BENCH_YTDLP").expect("BENCH_YTDLP"));
        for chunk in ["1M", "10M", "0"] {
            let flags: Vec<String> = if chunk == "0" {
                vec![]
            } else {
                vec!["--http-chunk-size".into(), chunk.into()]
            };
            let (tx, mut rx) = tokio::sync::mpsc::channel::<ProgressUpdate>(64);
            tokio::spawn(async move { while rx.recv().await.is_some() {} });
            let t = Instant::now();
            let r = omniget_core::core::ytdlp::download_video(
                &ytdlp,
                &url,
                &dir,
                None,
                tx,
                None,
                None,
                None,
                None,
                tokio_util::sync::CancellationToken::new(),
                None,
                8,
                false,
                &flags,
                None,
            )
            .await;
            let secs = t.elapsed().as_secs_f64();
            match r {
                Ok(res) => {
                    println!("| yt-dlp generic (http) | http-chunk-size {} | {secs:.2}s | {:.1} |", if chunk == "0" { "padrão do app (10M)" } else { chunk }, mib_s(res.file_size_bytes, secs));
                    let _ = std::fs::remove_file(res.file_path);
                }
                Err(e) => println!("| yt-dlp generic | chunk {chunk} | erro | {e} |"),
            }
        }
        return;
    }

    // modo raw / latency / rate / norange: varre segmentos e tamanhos
    let segments = env_list("BENCH_SEGMENTS", "1,2,4,8,16,32");
    let sizes = env_list("BENCH_SEGSIZE_MB", "4");
    // aquecimento (cache de disco do servidor)
    let _ = run_fetcher(&client, &url, dir.join("warm.bin"), 4, 4).await;
    for &mb in &sizes {
        for &s in &segments {
            let (bytes, secs) = run_fetcher(&client, &url, dir.join("b.bin"), s as usize, mb).await;
            println!("| HttpFetcher | {s} segmentos · {mb} MiB/segmento | {secs:.2}s | {:.1} |", mib_s(bytes, secs));
        }
    }
    // baseline: reqwest stream único sem o fetcher
    {
        use futures::StreamExt;
        let t = Instant::now();
        let mut stream = client.get(&url).send().await.unwrap().bytes_stream();
        let mut f = tokio::fs::File::create(dir.join("base.bin")).await.unwrap();
        let mut n = 0u64;
        while let Some(c) = stream.next().await {
            let c = c.unwrap();
            tokio::io::AsyncWriteExt::write_all(&mut f, &c).await.unwrap();
            n += c.len() as u64;
        }
        let secs = t.elapsed().as_secs_f64();
        println!("| reqwest cru (1 conexão, sem fetcher) | baseline | {secs:.2}s | {:.1} |", mib_s(n, secs));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
