use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use std::time::Duration;

use anyhow::anyhow;
use futures::StreamExt;
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::core::http_fetcher::{
    get_global_max_concurrent_segments, probe_remote, HttpFetcher, HttpFetcherConfig,
};
use crate::core::media_signature::{looks_like_html, sniff_media_format};
use crate::models::progress::ProgressUpdate;

const CHUNK_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_RETRIES: u32 = 3;
const CHUNK_SIZE: u64 = 10 * 1024 * 1024;
const CHUNK_THRESHOLD: u64 = 10 * 1024 * 1024;
const MAX_PARALLEL: usize = 12;
const MAX_PER_HOST: usize = 16;
/// Bytes read back from the finished file to tell media from an error page.
const SNIFF_BYTES: usize = 512;

fn host_semaphores() -> &'static tokio::sync::Mutex<HashMap<String, Arc<Semaphore>>> {
    static MAP: OnceLock<tokio::sync::Mutex<HashMap<String, Arc<Semaphore>>>> = OnceLock::new();
    MAP.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

pub async fn get_host_semaphore(url: &str) -> Arc<Semaphore> {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default();
    let mut map = host_semaphores().lock().await;
    map.entry(host)
        .or_insert_with(|| Arc::new(Semaphore::new(MAX_PER_HOST)))
        .clone()
}

struct ProbeResult {
    content_length: Option<u64>,
    accept_ranges: bool,
}

pub async fn download_direct(
    client: &reqwest::Client,
    url: &str,
    output: &Path,
    progress_tx: mpsc::Sender<ProgressUpdate>,
    cancel: Option<&CancellationToken>,
) -> anyhow::Result<u64> {
    download_direct_with_headers(client, url, output, progress_tx, None, cancel).await
}

pub async fn download_direct_with_headers(
    client: &reqwest::Client,
    url: &str,
    output: &Path,
    progress_tx: mpsc::Sender<ProgressUpdate>,
    headers: Option<reqwest::header::HeaderMap>,
    cancel: Option<&CancellationToken>,
) -> anyhow::Result<u64> {
    let mut last_err = None;
    // Two independent budgets, both monotonic, so the loop always terminates:
    // `attempt` counts the ordinary retries and `forbidden_retries` counts the
    // 403 ladder. The ladder gets its own counter so a couple of transient
    // network failures cannot eat the escalation steps before they run, and it
    // is a local — the count is per download request, never process-wide.
    let mut attempt: u32 = 0;
    let mut forbidden_retries: u32 = 0;
    let mut requests_made: u32 = 0;
    let mut effective_headers = headers;

    while attempt < MAX_RETRIES {
        if let Some(token) = cancel {
            if token.is_cancelled() {
                return Err(anyhow!("Download cancelled"));
            }
        }

        if requests_made > 0 {
            let base = 1000 * (requests_made as u64);
            let jitter = rand::random::<u64>() % (base / 2 + 1);
            tokio::time::sleep(Duration::from_millis(base + jitter)).await;
        }

        requests_made += 1;
        match download_attempt(
            client,
            url,
            output,
            &progress_tx,
            effective_headers.clone(),
            cancel,
        )
        .await
        {
            Ok(bytes) => return Ok(bytes),
            Err(e) => {
                // A 403 on the first try is often just a CDN that wants the
                // request to look more like a browser one. Climb the ladder
                // before letting `is_fatal_error` end the download: at most
                // two extra requests, and only ever two, because the step
                // function runs dry after that.
                if is_forbidden_error(&e) {
                    if let Some(step) = forbidden_retry_step(forbidden_retries) {
                        tracing::warn!("[direct] HTTP 403; retrying with {}", step.describe());
                        effective_headers = Some(headers_for_forbidden_retry(
                            effective_headers.as_ref(),
                            step,
                        ));
                        forbidden_retries += 1;
                        last_err = Some(e);
                        // Deliberately does not consume an ordinary retry.
                        continue;
                    }
                }
                if is_fatal_error(&e) {
                    let _ = std::fs::remove_file(&part_path_for(output));
                    return Err(e);
                }
                tracing::warn!(
                    "[direct] attempt {}/{} failed: {}",
                    attempt + 1,
                    MAX_RETRIES,
                    e
                );
                last_err = Some(e);
                attempt += 1;
            }
        }
    }

    let _ = std::fs::remove_file(&part_path_for(output));
    Err(last_err.unwrap_or_else(|| anyhow!("Download failed after {} attempts", MAX_RETRIES)))
}

fn part_path_for(output: &Path) -> PathBuf {
    let mut part = output.as_os_str().to_owned();
    part.push(".part");
    PathBuf::from(part)
}

fn is_fatal_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    for code in &[
        "HTTP 400", "HTTP 401", "HTTP 403", "HTTP 404", "HTTP 405", "HTTP 410", "HTTP 451",
    ] {
        if msg.contains(code) {
            return true;
        }
    }
    if msg.contains("HTML instead of media") {
        return true;
    }
    if msg.contains("cancelled") {
        return true;
    }
    false
}

/// A 403 is the one fatal code worth one more look: plenty of CDNs answer it
/// to a request that does not smell like a browser and then serve the file
/// happily to the very same URL with a couple of extra headers.
fn is_forbidden_error(err: &anyhow::Error) -> bool {
    err.to_string().contains("HTTP 403")
}

/// The rungs of the 403 ladder, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForbiddenRetryStep {
    /// Ask for the whole body as a range. A browser media element always does.
    RangeOnly,
    /// Same, plus the `sec-fetch-*` hints a media request carries.
    RangeAndFetchHints,
}

impl ForbiddenRetryStep {
    fn describe(self) -> &'static str {
        match self {
            ForbiddenRetryStep::RangeOnly => "Range: bytes=0-",
            ForbiddenRetryStep::RangeAndFetchHints => "Range + sec-fetch hints",
        }
    }
}

/// Which rung to try next, given how many the caller already burned.
///
/// `None` ends the ladder — that is what stops the retry loop from spinning,
/// since the caller only ever climbs while this returns `Some`.
fn forbidden_retry_step(retries_done: u32) -> Option<ForbiddenRetryStep> {
    match retries_done {
        0 => Some(ForbiddenRetryStep::RangeOnly),
        1 => Some(ForbiddenRetryStep::RangeAndFetchHints),
        _ => None,
    }
}

/// Caller headers plus whatever the given rung adds.
fn headers_for_forbidden_retry(
    base: Option<&reqwest::header::HeaderMap>,
    step: ForbiddenRetryStep,
) -> reqwest::header::HeaderMap {
    use reqwest::header::HeaderValue;

    let mut headers = base.cloned().unwrap_or_default();
    headers.insert(reqwest::header::RANGE, HeaderValue::from_static("bytes=0-"));
    if step == ForbiddenRetryStep::RangeAndFetchHints {
        headers.insert("sec-fetch-mode", HeaderValue::from_static("no-cors"));
        headers.insert("sec-fetch-site", HeaderValue::from_static("same-site"));
    }
    headers
}

/// `Err` só quando o host não responde de jeito nenhum; resposta estranha
/// (sem tamanho, sem Range) vira `Ok` com o que deu para saber.
async fn probe_url(
    client: &reqwest::Client,
    url: &str,
    headers: Option<&reqwest::header::HeaderMap>,
) -> anyhow::Result<ProbeResult> {
    match probe_remote(client, url, headers, Duration::from_secs(12)).await {
        Ok(p) => Ok(ProbeResult {
            content_length: p.content_length,
            accept_ranges: p.accept_ranges,
        }),
        Err(e) if e.to_string().starts_with("unreachable") => Err(e),
        Err(e) => {
            tracing::debug!("[direct] probe failed, streaming without Range: {}", e);
            Ok(ProbeResult {
                content_length: None,
                accept_ranges: false,
            })
        }
    }
}

async fn download_attempt(
    client: &reqwest::Client,
    url: &str,
    output: &Path,
    progress_tx: &mpsc::Sender<ProgressUpdate>,
    headers: Option<reqwest::header::HeaderMap>,
    cancel: Option<&CancellationToken>,
) -> anyhow::Result<u64> {
    let part_path = part_path_for(output);
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let probe = probe_url(client, url, headers.as_ref()).await?;

    let use_chunked =
        probe.accept_ranges && probe.content_length.is_some_and(|s| s > CHUNK_THRESHOLD);

    if use_chunked {
        match run_http_fetcher(client, url, output, progress_tx, headers.clone(), cancel).await {
            Ok(size) => return Ok(size),
            Err(fetch_err) => {
                if is_fatal_error(&fetch_err) {
                    return Err(fetch_err);
                }
                if fetch_err.to_string().contains("HTTP 429") {
                    // Não cair para o stream único: o host ainda está
                    // contando as conexões que acabaram de fechar. Espera e
                    // deixa a tentativa seguinte rodar com menos segmentos.
                    let _ = std::fs::remove_file(&part_path);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    return Err(fetch_err);
                }
                let _ = std::fs::remove_file(&part_path);
                tracing::warn!(
                    "[direct] http_fetcher failed, falling back to single stream: {}",
                    fetch_err
                );
                download_single_stream(
                    client,
                    url,
                    &part_path,
                    0,
                    probe.content_length,
                    progress_tx,
                    headers,
                    cancel,
                )
                .await?;
            }
        }
    } else {
        let existing = match std::fs::metadata(&part_path) {
            Ok(m) if m.len() > 0 && probe.accept_ranges => m.len(),
            _ => 0,
        };
        download_single_stream(
            client,
            url,
            &part_path,
            existing,
            probe.content_length,
            progress_tx,
            headers,
            cancel,
        )
        .await?;
    }

    if let Some(expected) = probe.content_length {
        let actual = std::fs::metadata(&part_path)?.len();
        if expected > 0 && actual != expected {
            let _ = std::fs::remove_file(&part_path);
            return Err(anyhow!(
                "Size mismatch: expected {} bytes, got {}",
                expected,
                actual
            ));
        }
    }

    if let Err(e) = reject_html_masquerading_as_media(&part_path) {
        let _ = std::fs::remove_file(&part_path);
        return Err(e);
    }

    std::fs::rename(&part_path, output)?;
    let _ = progress_tx.send(ProgressUpdate::percent(100.0)).await;

    let size = std::fs::metadata(output)?.len();
    Ok(size)
}

async fn run_http_fetcher(
    client: &reqwest::Client,
    url: &str,
    output: &Path,
    progress_tx: &mpsc::Sender<ProgressUpdate>,
    headers: Option<reqwest::header::HeaderMap>,
    cancel: Option<&CancellationToken>,
) -> anyhow::Result<u64> {
    // B35: o numero de Config vira **teto**, nao valor fixo. Um CDN que estrangula
    // com 8 conexoes entrega mais com 4, e so medindo da para saber qual e o caso
    // deste host. Sem historico, o comportamento e identico ao anterior.
    let teto = get_global_max_concurrent_segments()
        .unwrap_or(MAX_PARALLEL)
        .clamp(1, 32);
    let host = host_of(url);
    let concurrent = match &host {
        Some(h) => concurrency_tuner()
            .lock()
            .await
            .suggest(h, teto as u32, teto as u32) as usize,
        None => teto,
    };
    let inicio = std::time::Instant::now();
    let _active = super::adaptive_concurrency::ActiveDownloadGuard::new();
    let cfg = HttpFetcherConfig {
        min_size_for_chunked: 0,
        concurrent_segments: concurrent,
        segment_size_hint: CHUNK_SIZE,
        ..Default::default()
    };

    let mut fetcher =
        HttpFetcher::new(client.clone(), url.to_string(), output.to_path_buf()).with_config(cfg);
    if let Some(h) = headers {
        fetcher = fetcher.with_headers(h);
    }
    if let Some(c) = cancel {
        fetcher = fetcher.with_cancel(c.clone());
    }
    let result = match fetcher.download(progress_tx.clone()).await {
        Ok(r) => r,
        Err(e) => {
            // 429 com N conexões é o host dizendo "menos": registra para o
            // tuner cortar pela metade na próxima tentativa (o Hetzner speed
            // server devolveu 429 três vezes seguidas com 8 segmentos e o
            // retry cego repetia os mesmos 8).
            if e.to_string().contains("HTTP 429") {
                if let Some(h) = &host {
                    concurrency_tuner().lock().await.record_rate_limit(h);
                    tracing::warn!(
                        "[direct] {} rate-limited with {} segments; next attempt uses fewer",
                        h,
                        concurrent
                    );
                }
            }
            return Err(e);
        }
    };

    // So conta como amostra o que durou o suficiente para a medida significar
    // alguma coisa: um arquivo de 200 KB mede latencia, nao vazao.
    if let Some(h) = host {
        let secs = inicio.elapsed().as_secs_f64();
        if secs >= 1.0 && result.bytes_written > 0 {
            let bps = result.bytes_written as f64 / secs;
            concurrency_tuner()
                .lock()
                .await
                .record(&h, concurrent as u32, bps);
        }
    }

    Ok(result.bytes_written)
}

/// Historico de vazao por host, para o B35.
///
/// `tokio::sync::Mutex` e nao `std::sync::Mutex`: isto e travado dentro de
/// codigo async.
fn concurrency_tuner() -> &'static tokio::sync::Mutex<super::adaptive_concurrency::ConcurrencyTuner>
{
    super::adaptive_concurrency::global()
}

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()?
        .host_str()
        .map(|h| h.to_ascii_lowercase())
}

/// Last gate before the `.part` becomes the real file.
///
/// Only an HTML page is rejected. Sniffing the *positive* case would mean
/// failing every container we cannot recognise — subtitles, images, archives,
/// PDFs all go through this same path — so the rule is the conservative one:
/// no known media signature **and** it opens like a document. A CDN error page
/// served as `200 OK` is exactly that; a `.srt` is not.
///
/// The content-type check in `download_single_stream` only sees the header,
/// which a misconfigured CDN may set to `application/octet-stream` while the
/// body is still an error page. This reads the bytes that actually landed.
fn reject_html_masquerading_as_media(part_path: &Path) -> anyhow::Result<()> {
    let head = read_head(part_path, SNIFF_BYTES)?;
    if sniff_media_format(&head).is_none() && looks_like_html(&head) {
        return Err(anyhow!(
            "Server returned HTML instead of media — the link may have expired or needs a login"
        ));
    }
    Ok(())
}

/// First `max` bytes of a file, or fewer if the file is shorter.
fn read_head(path: &Path, max: usize) -> anyhow::Result<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buf = vec![0u8; max];
    let mut filled = 0usize;
    while filled < max {
        match file.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    buf.truncate(filled);
    Ok(buf)
}

#[allow(clippy::too_many_arguments)]
async fn download_single_stream(
    client: &reqwest::Client,
    url: &str,
    part_path: &Path,
    existing_bytes: u64,
    total_size: Option<u64>,
    progress_tx: &mpsc::Sender<ProgressUpdate>,
    headers: Option<reqwest::header::HeaderMap>,
    cancel: Option<&CancellationToken>,
) -> anyhow::Result<()> {
    let mut request = client.get(url);
    if let Some(mut h) = headers {
        if existing_bytes > 0 {
            // The 403 ladder may have added `Range: bytes=0-`; a resume needs
            // its own range, and `RequestBuilder::header` appends, so leaving
            // both in would send two Range headers on the same request.
            h.remove(reqwest::header::RANGE);
        }
        request = request.headers(h);
    }

    if existing_bytes > 0 {
        if let Some(total) = total_size {
            if existing_bytes >= total {
                return Ok(());
            }
        }
        request = request.header("Range", format!("bytes={}-", existing_bytes));
    }

    let response = request.send().await?;

    let mut offset = 0u64;
    if existing_bytes > 0 {
        if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            offset = existing_bytes;
        } else if response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            let _ = std::fs::remove_file(part_path);
            return Err(anyhow!("Range not satisfiable, restarting"));
        } else if !response.status().is_success() {
            return Err(anyhow!("HTTP {} downloading {}", response.status(), url));
        }
    } else if !response.status().is_success() {
        return Err(anyhow!("HTTP {} downloading {}", response.status(), url));
    }

    if let Some(ct) = response.headers().get("content-type") {
        if let Ok(ct_str) = ct.to_str() {
            if ct_str.contains("text/html") {
                return Err(anyhow!(
                    "Server returned HTML instead of media — URL may have expired"
                ));
            }
        }
    }

    use std::io::Write;
    let raw_file = if offset > 0 {
        std::fs::OpenOptions::new().append(true).open(part_path)?
    } else {
        std::fs::File::create(part_path)?
    };

    let mut file = std::io::BufWriter::with_capacity(256 * 1024, raw_file);
    let mut downloaded = offset;
    let mut stream = response.bytes_stream();

    let mut last_emit = std::time::Instant::now();
    let mut speed_anchor_bytes = downloaded;
    let mut speed_anchor_time = std::time::Instant::now();
    let mut speed_ema: f64 = 0.0;

    loop {
        if let Some(token) = cancel {
            if token.is_cancelled() {
                file.flush()?;
                return Err(anyhow!("Download cancelled"));
            }
        }

        let chunk_result = tokio::time::timeout(CHUNK_TIMEOUT, stream.next()).await;
        match chunk_result {
            Ok(Some(Ok(chunk))) => {
                file.write_all(&chunk)
                    .map_err(|e| anyhow!("Write error (disk full?): {}", e))?;
                downloaded += chunk.len() as u64;

                if last_emit.elapsed() >= std::time::Duration::from_millis(250) {
                    let dt = speed_anchor_time.elapsed().as_secs_f64();
                    if dt >= 0.2 {
                        let instant = (downloaded.saturating_sub(speed_anchor_bytes)) as f64 / dt;
                        speed_ema = if speed_ema > 0.0 {
                            speed_ema * 0.6 + instant * 0.4
                        } else {
                            instant
                        };
                        speed_anchor_bytes = downloaded;
                        speed_anchor_time = std::time::Instant::now();
                    }
                    let speed = (speed_ema > 0.0).then_some(speed_ema);
                    let (percent, eta) = match total_size {
                        Some(total) if total > 0 => {
                            let pct = (downloaded as f64 / total as f64) * 100.0;
                            let eta = speed.and_then(|s| {
                                (s > 0.0 && total > downloaded)
                                    .then(|| ((total - downloaded) as f64 / s) as u64)
                            });
                            (pct, eta)
                        }
                        _ => (
                            ((downloaded as f64 / (downloaded as f64 + 500_000.0)) * 100.0)
                                .min(95.0),
                            None,
                        ),
                    };
                    let _ = progress_tx
                        .send(ProgressUpdate::rich(
                            percent,
                            Some(downloaded),
                            total_size.filter(|t| *t > 0),
                            speed,
                            eta,
                        ))
                        .await;
                    last_emit = std::time::Instant::now();
                }
            }
            Ok(Some(Err(e))) => {
                file.flush()?;
                return Err(anyhow!("Download stream error: {}", e));
            }
            Ok(None) => break,
            Err(_) => {
                file.flush()?;
                return Err(anyhow!(
                    "Download timeout — no data received for 30 seconds"
                ));
            }
        }
    }

    file.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_path_appends_suffix() {
        let output = Path::new("video.mp4");
        let part = part_path_for(output);
        assert_eq!(part, PathBuf::from("video.mp4.part"));
    }

    #[test]
    fn part_path_no_extension() {
        let output = Path::new("video");
        let part = part_path_for(output);
        assert_eq!(part, PathBuf::from("video.part"));
    }

    #[test]
    fn part_path_nested() {
        let output = Path::new("downloads/curso/aula.mp4");
        let part = part_path_for(output);
        assert_eq!(part, PathBuf::from("downloads/curso/aula.mp4.part"));
    }

    #[test]
    fn is_fatal_http_400() {
        assert!(is_fatal_error(&anyhow!("HTTP 400 downloading url")));
    }

    #[test]
    fn is_fatal_http_401() {
        assert!(is_fatal_error(&anyhow!("HTTP 401 downloading url")));
    }

    #[test]
    fn is_fatal_http_403() {
        assert!(is_fatal_error(&anyhow!("HTTP 403 downloading url")));
    }

    #[test]
    fn is_fatal_http_404() {
        assert!(is_fatal_error(&anyhow!("HTTP 404 downloading url")));
    }

    #[test]
    fn is_fatal_html_response() {
        assert!(is_fatal_error(&anyhow!(
            "Server returned HTML instead of media — URL may have expired"
        )));
    }

    #[test]
    fn is_fatal_cancelled() {
        assert!(is_fatal_error(&anyhow!("Download cancelled")));
    }

    #[test]
    fn is_not_fatal_500() {
        assert!(!is_fatal_error(&anyhow!("HTTP 500 Internal Server Error")));
    }

    #[test]
    fn is_not_fatal_502() {
        assert!(!is_fatal_error(&anyhow!("HTTP 502 Bad Gateway")));
    }

    #[test]
    fn is_not_fatal_timeout() {
        assert!(!is_fatal_error(&anyhow!("connection timed out")));
    }

    #[test]
    fn is_not_fatal_network() {
        assert!(!is_fatal_error(&anyhow!("network error")));
    }

    #[test]
    fn chunk_count_for_12mb() {
        let total: u64 = 12 * 1024 * 1024;
        assert_eq!(total.div_ceil(CHUNK_SIZE), 2);
    }

    #[test]
    fn chunk_count_exact_boundary() {
        assert_eq!(CHUNK_SIZE.div_ceil(CHUNK_SIZE), 1);
    }

    #[test]
    fn chunk_count_single_byte_over() {
        assert_eq!((CHUNK_SIZE + 1).div_ceil(CHUNK_SIZE), 2);
    }

    #[test]
    fn threshold_gte_chunk_size() {
        assert!(CHUNK_THRESHOLD >= CHUNK_SIZE);
    }

    #[test]
    fn host_e_a_chave_do_historico_de_vazao() {
        // O historico e por host: medir "cdn.exemplo.com" e aplicar em outro
        // dominio seria pior que nao medir.
        assert_eq!(
            host_of("https://CDN.Exemplo.com/a/b.mp4"),
            Some("cdn.exemplo.com".to_string()),
            "host tem que normalizar para caixa baixa, senao vira duas entradas"
        );
        assert_eq!(host_of("nao e url"), None);
    }

    #[test]
    fn forbidden_ladder_has_exactly_two_rungs_in_order() {
        assert_eq!(
            forbidden_retry_step(0),
            Some(ForbiddenRetryStep::RangeOnly),
            "first retry only re-sends with Range"
        );
        assert_eq!(
            forbidden_retry_step(1),
            Some(ForbiddenRetryStep::RangeAndFetchHints),
            "second retry adds the sec-fetch hints"
        );
        assert_eq!(forbidden_retry_step(2), None, "the ladder ends here");
        assert_eq!(forbidden_retry_step(99), None);
    }

    #[test]
    fn forbidden_ladder_terminates_and_then_403_is_fatal() {
        // The decision logic of the retry loop, without any network: keep
        // climbing while the ladder offers a rung, then fall through to the
        // fatal check. Bounded by construction — `forbidden_retry_step` runs
        // dry, so the loop cannot spin.
        let err = anyhow!("HTTP 403 downloading https://cdn.example/a.mp4");
        let mut forbidden_retries = 0u32;
        let mut requests = 1u32; // the first request already happened

        loop {
            if !is_forbidden_error(&err) {
                break;
            }
            match forbidden_retry_step(forbidden_retries) {
                Some(_) => {
                    forbidden_retries += 1;
                    requests += 1;
                }
                None => break,
            }
        }

        assert_eq!(forbidden_retries, 2, "exactly two extra tries");
        assert_eq!(requests, 3, "three requests in total, never more");
        assert!(
            is_fatal_error(&err),
            "once the ladder is spent a 403 is fatal again"
        );
    }

    #[test]
    fn only_403_climbs_the_ladder() {
        assert!(is_forbidden_error(&anyhow!("HTTP 403 downloading url")));
        for other in ["HTTP 400", "HTTP 401", "HTTP 404", "HTTP 410", "HTTP 451"] {
            let e = anyhow!("{} downloading url", other);
            assert!(
                !is_forbidden_error(&e),
                "{other} must stay fatal on the first try"
            );
            assert!(is_fatal_error(&e));
        }
    }

    #[test]
    fn forbidden_retry_headers_escalate_and_keep_the_caller_headers() {
        let mut base = reqwest::header::HeaderMap::new();
        base.insert(
            reqwest::header::REFERER,
            reqwest::header::HeaderValue::from_static("https://example.com/"),
        );

        let first = headers_for_forbidden_retry(Some(&base), ForbiddenRetryStep::RangeOnly);
        assert_eq!(first.get(reqwest::header::RANGE).unwrap(), "bytes=0-");
        assert!(first.get("sec-fetch-mode").is_none());
        assert_eq!(
            first.get(reqwest::header::REFERER).unwrap(),
            "https://example.com/",
            "the caller's headers survive the escalation"
        );

        let second =
            headers_for_forbidden_retry(Some(&first), ForbiddenRetryStep::RangeAndFetchHints);
        assert_eq!(second.get(reqwest::header::RANGE).unwrap(), "bytes=0-");
        assert_eq!(second.get("sec-fetch-mode").unwrap(), "no-cors");
        assert_eq!(second.get("sec-fetch-site").unwrap(), "same-site");
        assert_eq!(
            second.get_all(reqwest::header::RANGE).iter().count(),
            1,
            "insert, not append: never two Range headers"
        );
    }

    #[test]
    fn forbidden_retry_headers_work_without_caller_headers() {
        let h = headers_for_forbidden_retry(None, ForbiddenRetryStep::RangeAndFetchHints);
        assert_eq!(h.get(reqwest::header::RANGE).unwrap(), "bytes=0-");
        assert_eq!(h.get("sec-fetch-site").unwrap(), "same-site");
    }

    fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "omniget-direct-test-{}-{}",
            std::process::id(),
            name
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn html_error_page_is_rejected_before_rename() {
        let path = temp_file(
            "erro.mp4.part",
            b"<!DOCTYPE html>\n<html><body>Access denied</body></html>",
        );
        let err = reject_html_masquerading_as_media(&path).unwrap_err();
        assert!(
            err.to_string().contains("HTML instead of media"),
            "the message has to be the fatal one: {err}"
        );
        assert!(is_fatal_error(&err), "no point retrying an error page");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn real_media_passes_the_gate() {
        let mut mp4 = vec![0x00, 0x00, 0x00, 0x20];
        mp4.extend_from_slice(b"ftypisom");
        mp4.resize(2048, 0);
        let path = temp_file("ok.mp4.part", &mp4);
        assert!(reject_html_masquerading_as_media(&path).is_ok());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unsniffable_containers_are_never_rejected() {
        // The whole reason the gate only rejects HTML: subtitles, images and
        // archives share this code path and have no media signature at all.
        for (name, body) in [
            (
                "legenda.srt.part",
                b"1\n00:00:01,000 --> 00:00:02,000\nOi\n".as_slice(),
            ),
            (
                "capa.jpg.part",
                &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46][..],
            ),
            (
                "pack.zip.part",
                b"PK\x03\x04....................".as_slice(),
            ),
            ("doc.pdf.part", b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".as_slice()),
            ("vazio.bin.part", b"".as_slice()),
        ] {
            let path = temp_file(name, body);
            assert!(
                reject_html_masquerading_as_media(&path).is_ok(),
                "{name} must not be rejected by the HTML gate"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    #[test]
    fn read_head_stops_at_the_limit() {
        let path = temp_file("grande.bin.part", &vec![7u8; SNIFF_BYTES * 4]);
        assert_eq!(read_head(&path, SNIFF_BYTES).unwrap().len(), SNIFF_BYTES);
        let _ = std::fs::remove_file(&path);

        let short = temp_file("curto.bin.part", &[1, 2, 3]);
        assert_eq!(read_head(&short, SNIFF_BYTES).unwrap(), vec![1, 2, 3]);
        let _ = std::fs::remove_file(&short);
    }

    #[tokio::test]
    async fn sem_historico_o_teto_e_respeitado() {
        // Primeira vez num host: nada medido, entao o numero de Config vale.
        let tuner = super::super::adaptive_concurrency::ConcurrencyTuner::default();
        assert_eq!(tuner.suggest("novo.host", 8, 8), 8);
        // E o teto e teto mesmo, nao sugestao.
        assert_eq!(tuner.suggest("novo.host", 32, 4), 4);
    }
}
