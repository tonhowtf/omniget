//! Exportações: CSV para planilha/migração (Eagle, Notion, Are.na), JSON
//! completo, galeria HTML offline (busca, agrupada por seção, sem "More
//! ideas") e PDF com um pin por página.

use std::collections::HashMap;

use std::path::Path;

use super::api::Pin;

/// Garante JPEG (PNG/WebP passam pelo ffmpeg), para o PDF.
pub async fn to_jpeg(data: Vec<u8>, work: &Path, idx: usize) -> anyhow::Result<Vec<u8>> {
    super::super::slides::ensure_jpeg(data, work, idx).await
}

fn csv_cell(s: &str) -> String {
    let needs = s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r');
    if needs {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub const CSV_HEADER: &[&str] = &[
    "id", "pin_url", "title", "description", "alt_text", "link", "domain", "board", "section", "pinner", "creator", "created_at",
    "kind", "image_url", "width", "height", "video_url", "saves", "repins", "comments", "reactions", "promoted", "ai_labeled",
    "ai_keyword", "dominant_color", "local_files",
];

pub fn csv(pins: &[Pin], files: &HashMap<String, Vec<String>>) -> String {
    let mut out = String::new();
    out.push_str(&CSV_HEADER.join(","));
    out.push('\n');
    for p in pins {
        let img = p.image.as_ref().or(p.image_large.as_ref());
        let row = vec![
            p.id.clone(),
            p.url.clone(),
            p.title.clone(),
            p.description.clone(),
            p.alt_text.clone(),
            p.link.clone().unwrap_or_default(),
            p.domain.clone().unwrap_or_default(),
            p.board.as_ref().and_then(|b| b.name.clone()).unwrap_or_default(),
            p.section.clone().unwrap_or_default(),
            p.pinner.as_ref().and_then(|x| x.username.clone()).unwrap_or_default(),
            p.creator.as_ref().and_then(|x| x.username.clone()).unwrap_or_default(),
            p.created_at.clone().unwrap_or_default(),
            p.kind.clone(),
            img.map(|m| m.url.clone()).unwrap_or_default(),
            img.map(|m| m.width.to_string()).unwrap_or_default(),
            img.map(|m| m.height.to_string()).unwrap_or_default(),
            p.video.as_ref().and_then(|v| v.mp4.clone().or_else(|| v.hls.clone())).unwrap_or_default(),
            p.saves.to_string(),
            p.repins.to_string(),
            p.comments.to_string(),
            p.reactions.to_string(),
            p.is_promoted.to_string(),
            p.ai.labeled.to_string(),
            p.ai.keyword.clone().unwrap_or_default(),
            p.dominant_color.clone().unwrap_or_default(),
            files.get(&p.id).map(|f| f.join(" | ")).unwrap_or_default(),
        ];
        out.push_str(&row.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Caminho relativo à pasta da galeria (os arquivos ficam dentro dela).
fn rel(root: &str, file: &str) -> String {
    let r = root.trim_end_matches(['/', '\\']);
    if let Some(rest) = file.strip_prefix(r) {
        rest.trim_start_matches(['/', '\\']).replace('\\', "/")
    } else {
        file.to_string()
    }
}

/// Galeria HTML de arquivo único. `files` aponta para os arquivos locais;
/// sem arquivo local a imagem vem do CDN (736x).
pub fn html_gallery(title: &str, subtitle: &str, pins: &[Pin], files: &HashMap<String, Vec<String>>, root: &str) -> String {
    let mut sections: Vec<String> = Vec::new();
    for p in pins {
        let s = p.section.clone().unwrap_or_default();
        if !sections.contains(&s) {
            sections.push(s);
        }
    }
    sections.sort_by(|a, b| a.is_empty().cmp(&b.is_empty()).then(a.cmp(b)));
    let mut items = String::new();
    for p in pins {
        let local = files.get(&p.id).and_then(|f| {
            f.iter()
                .find(|x| {
                    let l = x.to_lowercase();
                    l.ends_with(".jpg") || l.ends_with(".jpeg") || l.ends_with(".png") || l.ends_with(".webp") || l.ends_with(".gif")
                })
                .cloned()
        });
        let video_local = files.get(&p.id).and_then(|f| f.iter().find(|x| x.to_lowercase().ends_with(".mp4")).cloned());
        let src = local
            .map(|f| rel(root, &f))
            .or_else(|| p.image_large.as_ref().map(|m| m.url.clone()))
            .or_else(|| p.thumb.clone())
            .unwrap_or_default();
        let sec = p.section.clone().unwrap_or_default();
        let text = format!("{} {} {}", p.title, p.description, p.alt_text).to_lowercase();
        let media = if let Some(v) = video_local {
            format!(r#"<video src="{}" controls muted loop playsinline poster="{}"></video>"#, esc(&rel(root, &v)), esc(&src))
        } else {
            format!(r#"<img src="{}" alt="{}" loading="lazy">"#, esc(&src), esc(&p.title))
        };
        let link = p
            .link
            .as_ref()
            .map(|l| format!(r#" · <a href="{}" target="_blank" rel="noopener">{}</a>"#, esc(l), esc(p.domain.as_deref().unwrap_or("site"))))
            .unwrap_or_default();
        let badges = format!(
            "{}{}{}",
            if p.kind == "video" { r#"<span class="b">vídeo</span>"# } else { "" },
            if p.is_promoted { r#"<span class="b warn">anúncio</span>"# } else { "" },
            if p.ai.labeled || p.ai.keyword_level >= 2 { r#"<span class="b ai">IA</span>"# } else { "" },
        );
        items.push_str(&format!(
            r#"<figure data-sec="{sec}" data-text="{text}" data-kind="{kind}">{media}<figcaption><strong>{title}</strong><span class="meta">{saves} saves{link}</span>{badges}<a class="open" href="{url}" target="_blank" rel="noopener">abrir no Pinterest</a></figcaption></figure>
"#,
            sec = esc(&sec),
            text = esc(&text),
            kind = esc(&p.kind),
            media = media,
            title = esc(&p.title),
            saves = p.saves,
            link = link,
            badges = badges,
            url = esc(&p.url),
        ));
    }
    let section_buttons: String = sections
        .iter()
        .map(|s| format!(r#"<button data-sec="{0}">{1}</button>"#, esc(s), if s.is_empty() { "sem seção" } else { s }))
        .collect();
    format!(
        r##"<!doctype html><html lang="pt"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{title}</title>
<style>
:root{{color-scheme:light dark;--bg:#fafafa;--fg:#111;--dim:#666;--card:#fff;--line:#e5e5e5}}
@media(prefers-color-scheme:dark){{:root{{--bg:#111;--fg:#eee;--dim:#999;--card:#1c1c1e;--line:#2c2c2e}}}}
*{{box-sizing:border-box}}body{{margin:0;font:15px/1.4 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;background:var(--bg);color:var(--fg)}}
header{{position:sticky;top:0;z-index:2;background:color-mix(in srgb,var(--bg) 85%,transparent);backdrop-filter:blur(12px);border-bottom:1px solid var(--line);padding:14px 20px;display:flex;flex-wrap:wrap;gap:10px;align-items:center}}
h1{{font-size:20px;margin:0 12px 0 0}}header .sub{{color:var(--dim);font-size:13px;margin-right:auto}}
input[type=search]{{padding:8px 12px;border:1px solid var(--line);border-radius:10px;background:var(--card);color:var(--fg);min-width:240px}}
button{{padding:6px 10px;border:1px solid var(--line);border-radius:999px;background:var(--card);color:var(--fg);cursor:pointer;font-size:13px}}button.on{{background:#e60023;color:#fff;border-color:#e60023}}
main{{columns:5 220px;column-gap:14px;padding:18px 20px}}@media(max-width:700px){{main{{columns:2 150px}}}}
figure{{break-inside:avoid;margin:0 0 14px;background:var(--card);border-radius:16px;overflow:hidden;box-shadow:0 1px 2px rgba(0,0,0,.08)}}figure.hide{{display:none}}
figure img,figure video{{display:block;width:100%;height:auto}}figcaption{{padding:8px 10px 10px;display:flex;flex-direction:column;gap:3px;font-size:13px}}
figcaption strong{{font-weight:600;line-height:1.25}}.meta{{color:var(--dim);font-size:12px}}.meta a{{color:inherit}}
.b{{display:inline-block;font-size:11px;padding:1px 7px;border-radius:999px;background:var(--line);margin-right:4px}}.b.warn{{background:#f5d0a9}}.b.ai{{background:#d8c8ff}}
a.open{{color:#e60023;text-decoration:none;font-size:12px}}a.open:hover{{text-decoration:underline}}
.count{{color:var(--dim);font-size:13px}}
</style></head><body>
<header><h1>{title}</h1><span class="sub">{subtitle}</span><input type="search" id="q" placeholder="Buscar nos pins…"><span class="count" id="c"></span></header>
<div style="padding:10px 20px 0;display:flex;flex-wrap:wrap;gap:6px">{secs}</div>
<main id="g">
{items}</main>
<script>
const q=document.getElementById('q'),c=document.getElementById('c'),figs=[...document.querySelectorAll('figure')],btns=[...document.querySelectorAll('button[data-sec]')];let sec=null;
function apply(){{const t=q.value.trim().toLowerCase();let n=0;for(const f of figs){{const ok=(!t||f.dataset.text.includes(t))&&(sec===null||f.dataset.sec===sec);f.classList.toggle('hide',!ok);if(ok)n++;}}c.textContent=n+' / '+figs.length;}}
q.addEventListener('input',apply);btns.forEach(b=>b.addEventListener('click',()=>{{sec=(sec===b.dataset.sec)?null:b.dataset.sec;btns.forEach(x=>x.classList.toggle('on',x.dataset.sec===sec));apply();}}));apply();
</script></body></html>"##,
        title = esc(title),
        subtitle = esc(subtitle),
        secs = if sections.len() > 1 { section_buttons } else { String::new() },
        items = items,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_escapes() {
        let p = Pin { id: "1".into(), title: "a, \"b\"".into(), ..Default::default() };
        let out = csv(&[p], &HashMap::new());
        assert!(out.lines().nth(1).unwrap().contains("\"a, \"\"b\"\"\""));
        assert!(out.starts_with("id,pin_url,title"));
    }

    #[test]
    fn gallery_has_items_and_sections() {
        let mut a = Pin { id: "1".into(), title: "One".into(), section: Some("Sala".into()), ..Default::default() };
        a.thumb = Some("https://i.pinimg.com/236x/x.jpg".into());
        let b = Pin { id: "2".into(), title: "Two".into(), ..Default::default() };
        let html = html_gallery("Board", "2 pins", &[a, b], &HashMap::new(), "/tmp/x");
        assert!(html.contains("<figure data-sec=\"Sala\""));
        assert!(html.contains("sem seção"));
        assert!(html.contains("One"));
    }

    #[test]
    fn relative_paths() {
        assert_eq!(rel("/a/b", "/a/b/c/d.jpg"), "c/d.jpg");
        assert_eq!(rel("C:\\a", "C:\\a\\x.jpg"), "x.jpg");
    }
}
