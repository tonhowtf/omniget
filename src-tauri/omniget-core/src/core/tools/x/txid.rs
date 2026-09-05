//! `x-client-transaction-id` (estudo 67). Porte do `xclid.py` do twscrape
//! (MIT), que por sua vez veio do XClientTransaction de @iSarabjitDhiman
//! (MIT). O X assina cada chamada com um hash que mistura a chave de
//! verificacao da pagina, um frame de uma animacao SVG escolhido por indices
//! escondidos num chunk JS (`ondemand.s` / `sign.o`), o metodo, o caminho e
//! o tempo. Sem o header a maioria das operacoes ainda responde; quando o X
//! exige, sem ele vem 404. Se qualquer etapa falhar, o cliente segue sem.

use base64::Engine;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct TxIdGen {
    vk_bytes: Vec<u8>,
    anim_key: String,
}

struct Cubic {
    curves: Vec<f64>,
}

impl Cubic {
    fn calculate(a: f64, b: f64, m: f64) -> f64 {
        3.0 * a * (1.0 - m) * (1.0 - m) * m + 3.0 * b * (1.0 - m) * m * m + m * m * m
    }

    fn get_value(&self, time: f64) -> f64 {
        let c = &self.curves;
        let (mut start, mut end, mut mid) = (0.0f64, 1.0f64, 0.0f64);
        if time <= 0.0 {
            let mut g = 0.0;
            if c[0] > 0.0 {
                g = c[1] / c[0];
            } else if c[1] == 0.0 && c[2] > 0.0 {
                g = c[3] / c[2];
            }
            return g * time;
        }
        if time >= 1.0 {
            let mut g = 0.0;
            if c[2] < 1.0 {
                g = (c[3] - 1.0) / (c[2] - 1.0);
            } else if c[2] == 1.0 && c[0] < 1.0 {
                g = (c[1] - 1.0) / (c[0] - 1.0);
            }
            return 1.0 + g * (time - 1.0);
        }
        while start < end {
            mid = (start + end) / 2.0;
            let x_est = Self::calculate(c[0], c[2], mid);
            if (time - x_est).abs() < 0.00001 {
                return Self::calculate(c[1], c[3], mid);
            }
            if x_est < time {
                start = mid;
            } else {
                end = mid;
            }
        }
        Self::calculate(c[1], c[3], mid)
    }
}

fn interpolate(from: &[f64], to: &[f64], f: f64) -> Vec<f64> {
    from.iter()
        .zip(to)
        .map(|(a, b)| a * (1.0 - f) + b * f)
        .collect()
}

fn rotation_matrix(deg: f64) -> [f64; 4] {
    let rad = deg.to_radians();
    [rad.cos(), -rad.sin(), rad.sin(), rad.cos()]
}

fn solve(value: f64, min: f64, max: f64, rounding: bool) -> f64 {
    let r = value * (max - min) / 255.0 + min;
    if rounding {
        r.floor()
    } else {
        (r * 100.0).round() / 100.0
    }
}

fn hex_char(n: i64) -> char {
    if n > 9 {
        (b'A' + (n - 10) as u8) as char
    } else {
        (b'0' + n as u8) as char
    }
}

fn float_to_hex(mut x: f64) -> String {
    let mut result: Vec<char> = Vec::new();
    let mut quotient = x.trunc() as i64;
    let mut fraction = x - quotient as f64;
    while quotient > 0 {
        quotient = (x / 16.0).trunc() as i64;
        let remainder = (x - (quotient as f64) * 16.0).trunc() as i64;
        result.insert(0, hex_char(remainder));
        x = quotient as f64;
    }
    if fraction == 0.0 {
        return result.into_iter().collect();
    }
    result.push('.');
    while fraction > 0.0 {
        fraction *= 16.0;
        let integer = fraction.trunc() as i64;
        fraction -= integer as f64;
        result.push(hex_char(integer));
    }
    result.into_iter().collect()
}

fn calc_anim_key(frames: &[f64], target_time: f64) -> String {
    let from_color = [frames[0], frames[1], frames[2], 1.0];
    let to_color = [frames[3], frames[4], frames[5], 1.0];
    let to_rotation = solve(frames[6], 60.0, 360.0, true);
    let curves: Vec<f64> = frames[7..]
        .iter()
        .enumerate()
        .map(|(i, x)| solve(*x, if i % 2 == 1 { -1.0 } else { 0.0 }, 1.0, false))
        .collect();
    let val = Cubic { curves }.get_value(target_time);
    let color: Vec<f64> = interpolate(&from_color, &to_color, val)
        .into_iter()
        .map(|c| c.clamp(0.0, 255.0))
        .collect();
    let rotation = interpolate(&[0.0], &[to_rotation], val);
    let matrix = rotation_matrix(rotation[0]);
    let mut parts: Vec<String> = color[..3]
        .iter()
        .map(|c| format!("{:x}", c.round() as i64))
        .collect();
    for value in matrix {
        let mut rounded = (value * 100.0).round() / 100.0;
        if rounded < 0.0 {
            rounded = -rounded;
        }
        let hex = float_to_hex(rounded);
        parts.push(if hex.starts_with('.') {
            format!("0{}", hex).to_lowercase()
        } else if !hex.is_empty() {
            hex
        } else {
            "0".to_string()
        });
    }
    parts.push("0".into());
    parts.push("0".into());
    parts.join("").replace(['.', '-'], "")
}

async fn page_text(
    http: &reqwest::Client,
    url: &str,
    cookie: Option<&str>,
) -> anyhow::Result<String> {
    let mut req = http.get(url);
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    let text = req.send().await?.error_for_status()?.text().await?;
    if !text.contains(">document.location =") {
        return Ok(text);
    }
    let next = text
        .split("document.location = \"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap_or("")
        .to_string();
    let mut req = http.get(&next);
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    let text = req.send().await?.error_for_status()?.text().await?;
    if !text.contains("action=\"https://x.com/x/migrate\" method=\"post\"") {
        return Ok(text);
    }
    let re = regex::Regex::new(r#"<input[^>]*name="([^"]+)"[^>]*value="([^"]*)""#).unwrap();
    let form: Vec<(String, String)> = re
        .captures_iter(&text)
        .map(|c| (c[1].to_string(), c[2].to_string()))
        .collect();
    let mut req = http.post("https://x.com/x/migrate").form(&form);
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    Ok(req.send().await?.error_for_status()?.text().await?)
}

fn verification_bytes(html: &str) -> anyhow::Result<Vec<u8>> {
    let re1 =
        regex::Regex::new(r#"<meta[^>]*name="twitter-site-verification"[^>]*content="([^"]+)""#)
            .unwrap();
    let re2 =
        regex::Regex::new(r#"<meta[^>]*content="([^"]+)"[^>]*name="twitter-site-verification""#)
            .unwrap();
    let content = re1
        .captures(html)
        .or_else(|| re2.captures(html))
        .map(|c| c[1].to_string())
        .ok_or_else(|| anyhow::anyhow!("chave de verificacao do X nao encontrada"))?;
    Ok(base64::engine::general_purpose::STANDARD.decode(content.as_bytes())?)
}

fn anim_frames(html: &str, vk: &[u8]) -> anyhow::Result<Vec<Vec<f64>>> {
    let re_svg =
        regex::Regex::new(r#"(?s)<svg[^>]*id="loading-x-anim-\d+"[^>]*>(.*?)</svg>"#).unwrap();
    let re_g = regex::Regex::new(r#"(?s)<g[^>]*>(.*?)</g>"#).unwrap();
    let re_path = regex::Regex::new(r#"<path[^>]*\sd="([^"]+)""#).unwrap();
    let mut ds: Vec<String> = Vec::new();
    for svg in re_svg.captures_iter(html) {
        let inner = &svg[1];
        let g = re_g
            .captures(inner)
            .map(|c| c[1].to_string())
            .unwrap_or_else(|| inner.to_string());
        let paths: Vec<String> = re_path
            .captures_iter(&g)
            .map(|c| c[1].trim().to_string())
            .collect();
        if let Some(d) = paths.get(1) {
            ds.push(d.clone());
        }
    }
    if ds.is_empty() {
        anyhow::bail!("animacao do X nao encontrada na pagina");
    }
    let idx = (vk.get(5).copied().unwrap_or(0) as usize) % ds.len();
    let d = &ds[idx];
    let body: String = d.chars().skip(9).collect();
    let re_num = regex::Regex::new(r"[^\d]+").unwrap();
    let mut rows = Vec::new();
    for part in body.split('C') {
        let nums: Vec<f64> = re_num
            .replace_all(part, " ")
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        rows.push(nums);
    }
    Ok(rows)
}

fn script_urls(html: &str) -> Vec<String> {
    let mut urls = Vec::new();
    for re in [
        regex::Regex::new(r"https://[\w.-]+/x-web/[\w./-]+\.js").unwrap(),
        regex::Regex::new(r"https://[\w.-]+/responsive-web/client-web(?:-legacy)?/[\w./~-]+\.js")
            .unwrap(),
    ] {
        for m in re.find_iter(html) {
            urls.push(m.as_str().to_string());
        }
    }
    let mut seen = std::collections::HashSet::new();
    urls.into_iter()
        .filter(|u| seen.insert(u.clone()))
        .collect()
}

fn indices_re() -> regex::Regex {
    regex::Regex::new(r"(?:\.{0,2}/)?[\w./-]*?\b(?:ondemand\.s|sign\.o)[\w.-]*\.js").unwrap()
}

fn join_url(base: &str, rel: &str) -> String {
    if rel.starts_with("http") {
        return rel.to_string();
    }
    url::Url::parse(base)
        .ok()
        .and_then(|b| b.join(rel).ok())
        .map(|u| u.to_string())
        .unwrap_or_else(|| rel.to_string())
}

async fn indices(
    http: &reqwest::Client,
    html: &str,
    cookie: Option<&str>,
) -> anyhow::Result<Vec<usize>> {
    let scripts = script_urls(html);
    let x_web: Vec<String> = scripts
        .iter()
        .filter(|u| u.contains("/x-web/"))
        .cloned()
        .collect();
    if x_web.iter().any(|u| u.contains("entry-client-logged-out")) {
        anyhow::bail!("o X serviu a versao deslogada da pagina");
    }
    let pool = if x_web.is_empty() {
        scripts.clone()
    } else {
        x_web
    };
    let re = indices_re();
    let mut url = pool.iter().find(|u| re.is_match(u)).cloned();
    if url.is_none() {
        let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(12));
        let mut tasks = Vec::new();
        for s in pool.into_iter().take(80) {
            let http = http.clone();
            let sem = sem.clone();
            tasks.push(tokio::spawn(async move {
                let _p = sem.acquire().await.ok()?;
                let text = http.get(&s).send().await.ok()?.text().await.ok()?;
                indices_re().find(&text).map(|m| join_url(&s, m.as_str()))
            }));
        }
        for t in tasks {
            if let Ok(Some(u)) = t.await {
                url = Some(u);
                break;
            }
        }
    }
    let url = url.ok_or_else(|| anyhow::anyhow!("chunk de assinatura do X nao encontrado"))?;
    let text = page_text(http, &url, cookie).await?;
    let re_idx = regex::Regex::new(r"\(\w\[(\d{1,2})\],\s*16\)").unwrap();
    let items: Vec<usize> = re_idx
        .captures_iter(&text)
        .filter_map(|c| c[1].parse().ok())
        .collect();
    if items.is_empty() {
        anyhow::bail!("indices de assinatura nao encontrados");
    }
    Ok(items)
}

impl TxIdGen {
    /// Precisa da sessao logada: o X so entrega a pagina com os indices para
    /// quem esta autenticado.
    pub async fn create(http: &reqwest::Client, cookie: Option<&str>) -> anyhow::Result<Self> {
        let html = page_text(http, "https://x.com/tesla", cookie).await?;
        let vk = verification_bytes(&html)?;
        let rows = anim_frames(&html, &vk)?;
        let idx = indices(http, &html, cookie).await?;
        let mut frame_time: i64 = 1;
        for i in idx.iter().skip(1) {
            frame_time *= (vk.get(*i).copied().unwrap_or(0) % 16) as i64;
        }
        let frame_time = ((frame_time as f64 / 10.0 + 0.5).floor() * 10.0) as i64;
        let frame_idx = (vk.get(idx[0]).copied().unwrap_or(0) % 16) as usize;
        let row = rows
            .get(frame_idx)
            .ok_or_else(|| anyhow::anyhow!("frame da animacao fora do intervalo"))?;
        if row.len() < 11 {
            anyhow::bail!("frame da animacao incompleto");
        }
        let anim_key = calc_anim_key(row, frame_time as f64 / 4096.0);
        Ok(Self {
            vk_bytes: vk,
            anim_key,
        })
    }

    pub fn calc(&self, method: &str, path: &str) -> String {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let ts = ((now_ms - 1_682_924_400_000) as f64 / 1000.0).floor() as i64;
        let ts_bytes: Vec<u8> = (0..4).map(|i| ((ts >> (i * 8)) & 0xff) as u8).collect();
        let payload = format!(
            "{}!{}!{}obfiowerehiring{}",
            method.to_uppercase(),
            path,
            ts,
            self.anim_key
        );
        let hash = Sha256::digest(payload.as_bytes());
        let mut pld: Vec<u8> = Vec::new();
        pld.extend_from_slice(&self.vk_bytes);
        pld.extend_from_slice(&ts_bytes);
        pld.extend_from_slice(&hash[..16]);
        pld.push(3);
        let num: u8 = rand::random();
        let mut out = vec![num];
        out.extend(pld.iter().map(|x| x ^ num));
        base64::engine::general_purpose::STANDARD
            .encode(&out)
            .trim_end_matches('=')
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_hex_matches_python() {
        assert_eq!(float_to_hex(1.0), "1");
        assert_eq!(float_to_hex(0.5), ".8");
        assert_eq!(float_to_hex(0.0), "");
        assert_eq!(float_to_hex(255.0), "FF");
    }

    #[test]
    fn anim_key_is_deterministic() {
        let frames: Vec<f64> = vec![
            10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 128.0, 10.0, 20.0, 30.0, 40.0,
        ];
        let a = calc_anim_key(&frames, 0.3);
        let b = calc_anim_key(&frames, 0.3);
        assert_eq!(a, b);
        assert!(!a.contains('.') && !a.contains('-'));
    }

    #[test]
    fn calc_produces_base64() {
        let g = TxIdGen {
            vk_bytes: vec![1, 2, 3, 4, 5, 6, 7, 8],
            anim_key: "abc".into(),
        };
        let id = g.calc("GET", "/i/api/graphql/x/y");
        assert!(id.len() > 20 && !id.ends_with('='));
    }
}
