//! Busca global (projeto, data, texto de mensagens e tool calls) e busca
//! dentro de uma sessão, com contexto de ~100 caracteres e o índice da
//! mensagem para o salto.
//!
//! A busca global não guarda cópia do texto: filtra as sessões pelo índice
//! (ferramenta, projeto, datas), descarta arquivos em que os bytes não
//! contêm o termo (sem parsear) e só então carrega e procura de verdade.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::model::{Role, Session, SessionMeta};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchQuery {
    pub text: String,
    pub tool: Option<String>,
    pub account: Option<String>,
    pub project: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    #[serde(default)]
    pub include_subagents: bool,
    /// Máximo de resultados (padrão 200).
    pub limit: Option<u32>,
    /// Máximo de trechos por sessão (padrão 5).
    pub per_session: Option<u32>,
    /// Procurar também em entradas e resultados de tools (padrão: sim).
    pub in_tools: Option<bool>,
    /// Não cancelar a busca global anterior que ainda roda (padrão: uma
    /// busca nova cancela a anterior, como numa caixa de busca).
    #[serde(default)]
    pub keep_previous: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub tool: String,
    pub session_id: String,
    pub title: Option<String>,
    pub project_path: Option<String>,
    pub ended: Option<String>,
    /// Índice da `Turn` na sessão (`None` quando bateu no título/projeto).
    pub turn: Option<usize>,
    pub role: Option<Role>,
    pub ts: Option<String>,
    /// `title` | `project` | `text` | `tool_name` | `tool_input` | `tool_result`.
    pub field: String,
    pub snippet: String,
    /// Posição do termo dentro de `snippet`, em caracteres.
    pub match_start: usize,
    pub match_len: usize,
    pub match_count: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SearchResult {
    pub hits: Vec<Hit>,
    pub sessions_scanned: u32,
    pub sessions_matched: u32,
    /// Parou antes de varrer tudo (limite atingido ou cancelada).
    pub truncated: bool,
    /// Uma busca mais nova a cancelou; o resultado é parcial.
    pub cancelled: bool,
}

/// Procura `needle` (case-insensitive) em `hay`: (nº de ocorrências,
/// trecho com ~100 caracteres em volta da primeira, início e tamanho do
/// termo no trecho).
pub fn find_ci(hay: &str, needle: &str) -> Option<(u32, String, usize, usize)> {
    if needle.is_empty() {
        return None;
    }
    let h: Vec<char> = hay.chars().collect();
    let hl: Vec<char> = h
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let n: Vec<char> = needle
        .chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect();
    if n.len() > hl.len() {
        return None;
    }
    let mut first = None;
    let mut count = 0u32;
    let mut i = 0;
    while i + n.len() <= hl.len() {
        if hl[i..i + n.len()] == n[..] {
            count += 1;
            if first.is_none() {
                first = Some(i);
            }
            i += n.len();
        } else {
            i += 1;
        }
    }
    let at = first?;
    let ctx = 100usize.saturating_sub(n.len()) / 2;
    let start = at.saturating_sub(ctx);
    let end = (at + n.len() + ctx).min(h.len());
    let mut snippet: String = h[start..end].iter().collect();
    snippet = snippet.replace(['\n', '\r', '\t'], " ");
    let mut offset = at - start;
    if start > 0 {
        snippet.insert(0, '…');
        offset += 1;
    }
    if end < h.len() {
        snippet.push('…');
    }
    Some((count, snippet, offset, n.len()))
}

fn hit_base(meta: &SessionMeta) -> Hit {
    Hit {
        tool: meta.tool.clone(),
        session_id: meta.id.clone(),
        title: meta.title.clone(),
        project_path: meta.project_path.clone(),
        ended: meta.ended.clone(),
        turn: None,
        role: None,
        ts: None,
        field: String::new(),
        snippet: String::new(),
        match_start: 0,
        match_len: 0,
        match_count: 0,
    }
}

/// Busca dentro de uma sessão já carregada.
pub fn search_in(s: &Session, needle: &str, in_tools: bool, limit: usize) -> Vec<Hit> {
    let mut out = Vec::new();
    for (i, t) in s.turns.iter().enumerate() {
        let mut fields: Vec<(&str, String)> = vec![("text", t.text.clone())];
        if in_tools {
            for c in &t.tool_calls {
                fields.push(("tool_name", c.name_raw.clone()));
                fields.push(("tool_input", c.input.to_string()));
                if let Some(r) = &c.result {
                    fields.push(("tool_result", r.clone()));
                }
            }
        }
        let mut best: Option<Hit> = None;
        let mut total = 0u32;
        for (field, text) in fields {
            if let Some((n, snip, st, len)) = find_ci(&text, needle) {
                total += n;
                if best.is_none() {
                    let mut h = hit_base(&s.meta);
                    h.turn = Some(i);
                    h.role = Some(t.role);
                    h.ts = Some(t.ts.clone());
                    h.field = field.into();
                    h.snippet = snip;
                    h.match_start = st;
                    h.match_len = len;
                    best = Some(h);
                }
            }
        }
        if let Some(mut h) = best {
            h.match_count = total;
            out.push(h);
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

// --- pré-filtro por bytes ----------------------------------------------------

/// Busca de bytes sem diferenciar maiúsculas (UTF-8), com a forma escapada
/// `\uXXXX` do JSON quando o termo não é ASCII. O motor do `regex` usa
/// SIMD para literais, bem mais rápido que comparar byte a byte.
struct Matcher {
    re: regex::bytes::Regex,
    /// Quantos bytes do fim de um bloco repetir no próximo.
    overlap: usize,
}

impl Matcher {
    fn new(needle_lower: &str) -> Option<Matcher> {
        let mut alts = vec![regex::escape(needle_lower)];
        if !needle_lower.is_ascii() {
            let esc = serde_json::to_string(needle_lower).unwrap_or_default();
            let esc = esc.trim_matches('"');
            // `serde_json` não escapa não-ASCII; escapa à mão como o JS.
            let mut u = String::new();
            for ch in esc.chars() {
                if ch.is_ascii() {
                    u.push(ch);
                } else {
                    let mut b = [0u16; 2];
                    for unit in ch.encode_utf16(&mut b) {
                        u.push_str(&format!("\\u{unit:04x}"));
                    }
                }
            }
            alts.push(regex::escape(&u));
        }
        let longest = alts.iter().map(|a| a.len()).max().unwrap_or(0);
        let re = regex::bytes::RegexBuilder::new(&alts.join("|"))
            .case_insensitive(true)
            .build()
            .ok()?;
        Some(Matcher {
            re,
            overlap: longest * 4 + 16,
        })
    }

    /// O arquivo contém o termo? Lê em blocos num buffer reaproveitado.
    fn file_matches(&self, path: &Path, buf: &mut Vec<u8>, stop: &dyn Fn() -> bool) -> bool {
        use std::io::Read;
        const CHUNK: usize = 4 << 20;
        let Ok(mut f) = std::fs::File::open(path) else {
            return false;
        };
        if buf.len() < CHUNK + self.overlap {
            buf.resize(CHUNK + self.overlap, 0);
        }
        let mut filled = 0usize;
        loop {
            let mut eof = false;
            while filled < buf.len() {
                match f.read(&mut buf[filled..]) {
                    Ok(0) => {
                        eof = true;
                        break;
                    }
                    Ok(n) => filled += n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => {
                        eof = true;
                        break;
                    }
                }
            }
            if self.re.is_match(&buf[..filled]) {
                return true;
            }
            if eof || stop() {
                return false;
            }
            let keep = self.overlap.min(filled);
            buf.copy_within(filled - keep..filled, 0);
            filled = keep;
        }
    }
}

/// Fonte de um arquivo por sessão em texto (dá para pré-filtrar os bytes).
fn raw_file(meta: &SessionMeta) -> bool {
    let ext = meta
        .source
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    (ext == "jsonl" || ext == "json") && meta.source.is_file()
}

/// Texto pesquisável guardado no índice para fontes sem arquivo de texto
/// (banco, `.zst`): `(ferramenta, id)` → (impressão digital, texto).
pub type SearchTexts = HashMap<(String, String), (String, Vec<u8>)>;
/// Texto novo para gravar no índice: chave, impressão digital, texto.
pub type NewText = ((String, String), String, Vec<u8>);

/// Os mesmos campos que `search_in` olha, num bloco só.
pub fn searchable_text(s: &Session) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    for t in &s.turns {
        out.extend_from_slice(t.text.as_bytes());
        out.push(b'\n');
        for c in &t.tool_calls {
            out.extend_from_slice(c.name_raw.as_bytes());
            out.push(b'\n');
            out.extend_from_slice(c.input.to_string().as_bytes());
            out.push(b'\n');
            if let Some(r) = &c.result {
                out.extend_from_slice(r.as_bytes());
                out.push(b'\n');
            }
        }
    }
    out
}

/// Fonte de um arquivo por sessão em texto: fica de fora de `SearchTexts`.
pub fn needs_text(meta: &SessionMeta) -> bool {
    !raw_file(meta)
}

/// Impressão digital de uma sessão (do índice): muda quando ela muda.
pub fn fp_of(m: &SessionMeta) -> String {
    format!(
        "{}:{}:{}:{}",
        m.mtime_ms,
        m.turn_count,
        m.usage.total(),
        m.source.display()
    )
}

// --- caches ----------------------------------------------------------------

/// Sessões carregadas recentemente (o parse é a parte cara de uma busca):
/// digitar "ded", "dedu", "dedupe" reaproveita as mesmas. Orçamento em bytes
/// de texto; sai a mais antiga.
struct SessionCache {
    map: HashMap<(String, String), (String, Arc<Session>, usize)>,
    order: VecDeque<(String, String)>,
    bytes: usize,
}

const CACHE_BYTES: usize = 64 << 20;

/// Texto pesquisável das sessões de arquivo já parseadas: um arquivo cujo
/// JSON bate só em campos que a busca não olha (`cwd`, ids) deixa de ser
/// parseado de novo nas próximas buscas.
struct TextCache {
    map: HashMap<(String, String), (String, Arc<Vec<u8>>)>,
    order: VecDeque<(String, String)>,
    bytes: usize,
}

const TEXT_CACHE_BYTES: usize = 128 << 20;

static TEXT_CACHE: Mutex<Option<TextCache>> = Mutex::new(None);

fn text_get(key: &(String, String), fp: &str) -> Option<Arc<Vec<u8>>> {
    let g = TEXT_CACHE.lock().ok()?;
    let (f, b) = g.as_ref()?.map.get(key)?;
    (f == fp).then(|| b.clone())
}

fn text_put(key: (String, String), fp: String, body: Vec<u8>) {
    let size = body.len() + 64;
    if size > TEXT_CACHE_BYTES / 8 {
        return;
    }
    let Ok(mut g) = TEXT_CACHE.lock() else { return };
    let c = g.get_or_insert_with(|| TextCache {
        map: HashMap::new(),
        order: VecDeque::new(),
        bytes: 0,
    });
    if let Some((_, old)) = c.map.remove(&key) {
        c.bytes -= old.len() + 64;
        c.order.retain(|k| *k != key);
    }
    while c.bytes + size > TEXT_CACHE_BYTES {
        let Some(k) = c.order.pop_front() else { break };
        if let Some((_, b)) = c.map.remove(&k) {
            c.bytes -= b.len() + 64;
        }
    }
    c.bytes += size;
    c.order.push_back(key.clone());
    c.map.insert(key, (fp, Arc::new(body)));
}

static CACHE: Mutex<Option<SessionCache>> = Mutex::new(None);

fn session_bytes(s: &Session) -> usize {
    s.turns
        .iter()
        .map(|t| {
            t.text.len()
                + t.tool_calls
                    .iter()
                    .map(|c| {
                        c.name_raw.len() + c.result.as_ref().map(|r| r.len()).unwrap_or(0) + 64
                    })
                    .sum::<usize>()
                + 128
        })
        .sum()
}

fn cache_get(m: &SessionMeta, fp: &str) -> Option<Arc<Session>> {
    let g = CACHE.lock().ok()?;
    let c = g.as_ref()?;
    let (f, s, _) = c.map.get(&(m.tool.clone(), m.id.clone()))?;
    (f == fp).then(|| s.clone())
}

fn cache_put(m: &SessionMeta, fp: String, s: Arc<Session>) {
    let size = session_bytes(&s);
    if size > CACHE_BYTES / 4 {
        return;
    }
    let Ok(mut g) = CACHE.lock() else { return };
    let c = g.get_or_insert_with(|| SessionCache {
        map: HashMap::new(),
        order: VecDeque::new(),
        bytes: 0,
    });
    let key = (m.tool.clone(), m.id.clone());
    if let Some((_, _, old)) = c.map.remove(&key) {
        c.bytes -= old;
        c.order.retain(|k| *k != key);
    }
    while c.bytes + size > CACHE_BYTES {
        let Some(k) = c.order.pop_front() else { break };
        if let Some((_, _, b)) = c.map.remove(&k) {
            c.bytes -= b;
        }
    }
    c.bytes += size;
    c.order.push_back(key.clone());
    c.map.insert(key, (fp, s, size));
}

/// Sessões que não contêm o último termo buscado. Um termo novo que contém o
/// anterior ("dedu" → "dedupe") pula todas elas sem ler nada.
struct Negatives {
    needle_lower: String,
    in_tools: bool,
    fps: HashMap<(String, String), String>,
}

static NEGATIVES: Mutex<Option<Negatives>> = Mutex::new(None);

/// Cada busca global nova cancela a anterior que ainda roda (a caixa de
/// busca manda uma por tecla), salvo `SearchQuery.keep_previous`.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Busca global sobre `candidates` (já filtrados pelo índice, mais recentes
/// primeiro). `load` carrega a sessão completa de um meta.
pub fn search_global(
    candidates: Vec<SessionMeta>,
    q: &SearchQuery,
    load: &(dyn Fn(&SessionMeta) -> Option<Session> + Sync),
    texts: &SearchTexts,
) -> (SearchResult, Vec<NewText>) {
    let needle = q.text.trim().to_string();
    let limit = q.limit.unwrap_or(200).clamp(1, 5000) as usize;
    let per = q.per_session.unwrap_or(5).clamp(1, 100) as usize;
    let in_tools = q.in_tools.unwrap_or(true);
    let mut res = SearchResult::default();
    if needle.is_empty() {
        return (res, Vec::new());
    }
    let my_gen = if q.keep_previous {
        None
    } else {
        Some(GENERATION.fetch_add(1, Ordering::SeqCst) + 1)
    };
    let superseded = || my_gen.is_some_and(|g| GENERATION.load(Ordering::SeqCst) != g);
    let needle_lower = needle.to_lowercase();
    let Some(matcher) = Matcher::new(&needle_lower) else {
        return (res, Vec::new());
    };
    let new_texts: Mutex<Vec<NewText>> = Mutex::new(Vec::new());
    // Título e projeto batem sem abrir nada.
    let mut hits: Vec<Hit> = Vec::new();
    for m in &candidates {
        for (field, val) in [
            ("title", m.title.as_deref()),
            ("project", m.project_path.as_deref()),
        ] {
            if let Some((n, snip, st, len)) = val.and_then(|v| find_ci(v, &needle)) {
                let mut h = hit_base(m);
                h.field = field.into();
                h.snippet = snip;
                h.match_start = st;
                h.match_len = len;
                h.match_count = n;
                hits.push(h);
                break;
            }
        }
    }
    // Negativos herdados de um termo contido neste.
    let inherited: HashMap<(String, String), String> = NEGATIVES
        .lock()
        .ok()
        .and_then(|g| {
            g.as_ref()
                .filter(|n| {
                    n.in_tools == in_tools
                        && !n.needle_lower.is_empty()
                        && needle_lower.contains(&n.needle_lower)
                })
                .map(|n| n.fps.clone())
        })
        .unwrap_or_default();
    let queue = Mutex::new(candidates.iter());
    let found: Mutex<Vec<(usize, Hit)>> = Mutex::new(Vec::new());
    let counters = Mutex::new((0u32, 0u32));
    let negatives: Mutex<HashMap<(String, String), String>> = Mutex::new(HashMap::new());
    let full = AtomicBool::new(false);
    let cancelled = AtomicBool::new(false);
    let order: HashMap<(&str, &str), usize> = candidates
        .iter()
        .enumerate()
        .map(|(i, m)| ((m.tool.as_str(), m.id.as_str()), i))
        .collect();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(2, 12);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let mut buf: Vec<u8> = Vec::new();
                let gone = || {
                    if superseded() {
                        cancelled.store(true, Ordering::Relaxed);
                    }
                    cancelled.load(Ordering::Relaxed)
                };
                let stop = || gone() || full.load(Ordering::Relaxed);
                loop {
                    if stop() {
                        break;
                    }
                    let next = queue.lock().ok().and_then(|mut q| q.next());
                    let Some(m) = next else { break };
                    if let Ok(mut c) = counters.lock() {
                        c.0 += 1;
                    }
                    let key = (m.tool.clone(), m.id.clone());
                    let fp = fp_of(m);
                    if inherited.get(&key) == Some(&fp) {
                        if let Ok(mut n) = negatives.lock() {
                            n.insert(key, fp);
                        }
                        continue;
                    }
                    let cached = cache_get(m, &fp);
                    let raw = raw_file(m);
                    // Sem arquivo de texto: o texto pesquisável guardado no
                    // índice faz o papel do pré-filtro.
                    let stored = (!raw)
                        .then(|| texts.get(&key).filter(|(f, _)| *f == fp))
                        .flatten();
                    let mem_text = if raw && cached.is_none() {
                        text_get(&key, &fp)
                    } else {
                        None
                    };
                    let miss = if cached.is_some() {
                        false
                    } else if let Some(body) = &mem_text {
                        !matcher.re.is_match(body)
                    } else if raw {
                        !matcher.file_matches(&m.source, &mut buf, &gone)
                    } else if let Some((_, body)) = stored {
                        !matcher.re.is_match(body)
                    } else {
                        false
                    };
                    if miss {
                        if !gone() {
                            if let Ok(mut n) = negatives.lock() {
                                n.insert(key, fp);
                            }
                        }
                        continue;
                    }
                    let sess = match cached {
                        Some(s) => s,
                        None => {
                            let Some(s) = load(m) else { continue };
                            let s = Arc::new(s);
                            if raw && mem_text.is_none() {
                                text_put(key.clone(), fp.clone(), searchable_text(&s));
                            }
                            if !raw && stored.is_none() {
                                let body = searchable_text(&s);
                                let is_hit = matcher.re.is_match(&body);
                                if let Ok(mut nt) = new_texts.lock() {
                                    nt.push((key.clone(), fp.clone(), body));
                                }
                                if !is_hit {
                                    if let Ok(mut n) = negatives.lock() {
                                        n.insert(key, fp);
                                    }
                                    continue;
                                }
                            }
                            cache_put(m, fp.clone(), s.clone());
                            s
                        }
                    };
                    if gone() {
                        break;
                    }
                    let hs = search_in(&sess, &needle, in_tools, per);
                    if hs.is_empty() {
                        if let Ok(mut n) = negatives.lock() {
                            n.insert(key, fp);
                        }
                        continue;
                    }
                    if let Ok(mut c) = counters.lock() {
                        c.1 += 1;
                    }
                    let rank = order
                        .get(&(m.tool.as_str(), m.id.as_str()))
                        .copied()
                        .unwrap_or(usize::MAX);
                    if let Ok(mut f) = found.lock() {
                        f.extend(hs.into_iter().map(|h| (rank, h)));
                        if f.len() + hits.len() >= limit {
                            full.store(true, Ordering::Relaxed);
                        }
                    }
                }
            });
        }
    });
    let (sc, matched) = counters.into_inner().unwrap_or((0, 0));
    let was_cancelled = cancelled.load(Ordering::Relaxed);
    if !was_cancelled {
        if let Ok(mut g) = NEGATIVES.lock() {
            *g = Some(Negatives {
                needle_lower: needle_lower.clone(),
                in_tools,
                fps: negatives.into_inner().unwrap_or_default(),
            });
        }
    }
    let mut body = found.into_inner().unwrap_or_default();
    // Ordem da lista de candidatos (mais recentes primeiro), depois o turno.
    body.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.turn.cmp(&b.1.turn)));
    hits.extend(body.into_iter().map(|(_, h)| h));
    res.truncated = hits.len() > limit || full.load(Ordering::Relaxed) || was_cancelled;
    hits.truncate(limit);
    res.hits = hits;
    res.sessions_scanned = sc;
    res.sessions_matched = matched;
    res.cancelled = was_cancelled;
    (res, new_texts.into_inner().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_keeps_about_100_chars_around_the_match() {
        let text = format!("{}AGULHA{}", "a".repeat(200), "b".repeat(200));
        let (n, snip, st, len) = find_ci(&text, "agulha").unwrap();
        assert_eq!(n, 1);
        assert_eq!(len, 6);
        let chars: Vec<char> = snip.chars().collect();
        let found: String = chars[st..st + len].iter().collect();
        assert_eq!(found, "AGULHA");
        assert!(chars.len() <= 104);
        assert_eq!(find_ci("Ação e AÇÃO", "ação").unwrap().0, 2);
    }

    #[test]
    fn matcher_crosses_chunk_borders_and_json_escapes() {
        let dir = std::env::temp_dir().join(format!("omniget-search-m-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("s.jsonl");
        // Termo partido na fronteira de um bloco de 4 MiB.
        let mut body = vec![b'x'; (4 << 20) - 3];
        body.extend_from_slice(b"AGUlha fim");
        std::fs::write(&f, &body).unwrap();
        let mut buf = Vec::new();
        assert!(Matcher::new("agulha")
            .unwrap()
            .file_matches(&f, &mut buf, &|| false));
        assert!(!Matcher::new("palheiro")
            .unwrap()
            .file_matches(&f, &mut buf, &|| false));
        // Não-ASCII cru e escapado como o JSON.stringify faz.
        std::fs::write(&f, "{\"t\":\"FUNÇÃO\"}").unwrap();
        assert!(Matcher::new("função")
            .unwrap()
            .file_matches(&f, &mut buf, &|| false));
        std::fs::write(&f, "{\"t\":\"fun\\u00e7\\u00e3o\"}").unwrap();
        assert!(Matcher::new("função")
            .unwrap()
            .file_matches(&f, &mut buf, &|| false));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `cargo test --release -p omniget-core --lib real_prefilter_bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_prefilter_bench() {
        let home = dirs::home_dir().unwrap().join(".claude").join("projects");
        let mut files = Vec::new();
        for d in std::fs::read_dir(&home).unwrap().flatten() {
            if let Ok(rd) = std::fs::read_dir(d.path()) {
                for f in rd.flatten() {
                    if f.path().extension().and_then(|e| e.to_str()) == Some("jsonl") {
                        files.push(f.path());
                    }
                }
            }
        }
        let total: u64 = files
            .iter()
            .map(|f| std::fs::metadata(f).map(|m| m.len()).unwrap_or(0))
            .sum();
        println!("{} files {} MB", files.len(), total >> 20);
        let t = std::time::Instant::now();
        let mut n = 0usize;
        for f in &files {
            n += std::fs::read(f).map(|b| b.len()).unwrap_or(0);
        }
        println!("read only 1 thread {:?} ({n})", t.elapsed());
        for needle in ["zzqx-nada-aqui-9f3", "worktree", "função"] {
            let m = Matcher::new(needle).unwrap();
            let mut buf = Vec::new();
            let t = std::time::Instant::now();
            let hits = files
                .iter()
                .filter(|f| m.file_matches(f, &mut buf, &|| false))
                .count();
            println!("{needle:?} matcher 1 thread {:?} hits={hits}", t.elapsed());
            let t = std::time::Instant::now();
            let hits = files
                .iter()
                .filter(|f| {
                    let b = std::fs::read(f).unwrap_or_default();
                    crate::core::sessions::util::bytes_contains_ci(&b, needle.as_bytes())
                })
                .count();
            println!("{needle:?} naive 1 thread {:?} hits={hits}", t.elapsed());
        }
    }
}
