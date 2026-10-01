//! `omniget read ...`: the Read building blocks from the terminal. Works
//! without the desktop app: book metadata, legal catalogs, OPDS feeds,
//! text to EPUB, and MangaDex chapter lists.

use anyhow::{anyhow, bail};
use std::path::PathBuf;

use omniget_core::core::read::{epub_writer as w, extract, kosync, opds, sources};

use crate::output;

#[derive(clap::Subcommand)]
pub enum ReadCommand {
    #[command(about = "Metadata of a book or comic file (EPUB, MOBI/AZW3, FB2, CBZ, folder)")]
    Meta { file: PathBuf },
    #[command(
        about = "Search a legal catalog: gutenberg, standardebooks, openlibrary, mangadex, mangaplus"
    )]
    Search {
        query: String,
        #[arg(short, long, default_value = "gutenberg")]
        source: String,
        #[arg(short, long, help = "Language code (e.g. pt, en, pt-br)")]
        lang: Option<String>,
    },
    #[command(about = "List an OPDS catalog page (Atom or OPDS 2 JSON)")]
    Opds {
        url: String,
        #[arg(long)]
        user: Option<String>,
        #[arg(long)]
        password: Option<String>,
    },
    #[command(about = "Turn a TXT, Markdown or HTML file into an EPUB (no calibre needed)")]
    ToEpub {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        author: Option<String>,
        #[arg(long)]
        lang: Option<String>,
    },
    #[command(about = "Chapters of a MangaDex series (id or URL)")]
    Manga {
        id: String,
        #[arg(
            short,
            long,
            help = "Chapter languages, comma separated (e.g. en,pt-br)"
        )]
        lang: Option<String>,
    },
    #[command(about = "The KOReader document id (partial MD5) of a file")]
    Hash { file: PathBuf },
}

pub async fn execute(cmd: ReadCommand, json: bool) -> anyhow::Result<()> {
    match cmd {
        ReadCommand::Meta { file } => {
            let ex = extract::extract(&file).map_err(|e| anyhow!(e))?;
            let mut m = ex.meta;
            let toc_len = m.toc.len();
            if !json {
                m.toc.clear();
            }
            if json || output::is_json_mode() {
                output::print_json(&serde_json::json!({
                    "format": ex.format,
                    "meta": m,
                    "has_cover": ex.cover.is_some(),
                }));
            } else {
                println!("{}", m.title);
                if !m.authors.is_empty() {
                    println!("  by {}", m.authors.join(", "));
                }
                if !m.series.is_empty() {
                    println!(
                        "  series: {} {}",
                        m.series,
                        m.series_index.map(|i| format!("#{i}")).unwrap_or_default()
                    );
                }
                println!(
                    "  format: {:?}{}",
                    ex.format,
                    if m.rtl { " (right to left)" } else { "" }
                );
                for (k, v) in [
                    ("language", &m.language),
                    ("publisher", &m.publisher),
                    ("published", &m.published),
                ] {
                    if !v.is_empty() {
                        println!("  {k}: {v}");
                    }
                }
                if let Some(isbn) = m.isbn() {
                    println!("  isbn: {isbn}");
                }
                if let Some(p) = m.page_count {
                    println!("  pages: {p}");
                }
                if toc_len > 0 {
                    println!("  contents: {toc_len} entries");
                }
                println!("  cover: {}", if ex.cover.is_some() { "yes" } else { "no" });
            }
        }
        ReadCommand::Search {
            query,
            source,
            lang,
        } => {
            let c = sources::client();
            let langs: Vec<String> = lang.clone().into_iter().collect();
            let page = match source.as_str() {
                "gutenberg" => {
                    let url = sources::gutendex::search_url(
                        &query,
                        lang.as_deref().unwrap_or(""),
                        "",
                        "popular",
                    );
                    let cache = omniget_core::core::paths::app_data_dir()
                        .unwrap_or_default()
                        .join("read")
                        .join("cache")
                        .join("pg_catalog.csv.gz");
                    sources::gutendex::search_or_catalog(
                        &c,
                        &url,
                        &query,
                        lang.as_deref().unwrap_or(""),
                        &cache,
                    )
                    .await
                }
                "openlibrary" => {
                    let url = sources::openlibrary::search_url(
                        &query,
                        lang.as_deref().unwrap_or(""),
                        "ebooks",
                        1,
                    );
                    sources::openlibrary::search(&c, &url, 1).await
                }
                "standardebooks" => sources::standardebooks::browse(&c, &query, None).await,
                "mangadex" => {
                    let url = sources::mangadex::search_url(&query, &langs, &[], "", &[], &[], 0);
                    sources::mangadex::search(&c, &url, lang.as_deref().unwrap_or("en")).await
                }
                "mangaplus" => {
                    sources::mangaplus::browse(
                        &c,
                        &query,
                        "search",
                        lang.as_deref().unwrap_or("en"),
                    )
                    .await
                }
                other => bail!("unknown source {other}"),
            }
            .map_err(|e| anyhow!(e))?;
            if json || output::is_json_mode() {
                output::print_json(&page);
            } else {
                for h in &page.hits {
                    let fmts: Vec<&str> = h.downloads.iter().map(|d| d.format.as_str()).collect();
                    println!(
                        "{}  {}",
                        h.title,
                        if h.authors.is_empty() {
                            String::new()
                        } else {
                            format!("— {}", h.authors.join(", "))
                        }
                    );
                    println!(
                        "    key: {}{}",
                        h.key,
                        if fmts.is_empty() {
                            String::new()
                        } else {
                            format!("  files: {}", fmts.join(", "))
                        }
                    );
                    if let Some(d) = h.downloads.first() {
                        println!("    {}", d.url);
                    }
                }
                if let Some(t) = page.total {
                    println!("{} of {t}", page.hits.len());
                }
            }
        }
        ReadCommand::Opds {
            url,
            user,
            password,
        } => {
            let c = sources::client();
            let mut req = c.get(&url);
            if let Some(u) = &user {
                req = req.basic_auth(u, password.as_deref());
            }
            let r = req.send().await?;
            let base = r.url().to_string();
            let body = r.error_for_status()?.text().await?;
            let feed = opds::parse(&body, &base).map_err(|e| anyhow!(e))?;
            if json || output::is_json_mode() {
                output::print_json(&feed);
            } else {
                println!("{}", feed.title);
                for e in &feed.entries {
                    match &e.navigation {
                        Some(n) if e.acquisitions.is_empty() => {
                            println!("  [+] {}  → {n}", e.title)
                        }
                        _ => {
                            println!("  {}  {}", e.title, e.authors.join(", "));
                            for a in &e.acquisitions {
                                println!("      {} {}", a.kind, a.href);
                            }
                        }
                    }
                }
                if let Some(n) = &feed.next {
                    println!("next: {n}");
                }
            }
        }
        ReadCommand::ToEpub {
            input,
            output: out,
            title,
            author,
            lang,
        } => {
            let bytes = std::fs::read(&input)?;
            let text = w::decode_text(&bytes);
            let fallback = extract::title_from_name(&input);
            let ext = input
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let chapters = match ext.as_str() {
                "md" | "markdown" => w::markdown_chapters(&text, &fallback),
                "html" | "htm" | "xhtml" => vec![w::html_chapter(&text, &fallback)],
                _ => w::text_chapters(&text, &fallback),
            };
            let meta = w::EpubMeta {
                title: title.unwrap_or(fallback),
                authors: author.into_iter().collect(),
                language: lang.unwrap_or_default(),
                ..Default::default()
            };
            let out = out.unwrap_or_else(|| input.with_extension("epub"));
            w::write_reflowable(&out, &meta, &chapters, None).map_err(|e| anyhow!(e))?;
            println!("{} ({} chapters)", out.display(), chapters.len());
        }
        ReadCommand::Manga { id, lang } => {
            let id = id
                .trim_end_matches('/')
                .split("/title/")
                .nth(1)
                .map(|s| s.split('/').next().unwrap_or(s).to_string())
                .unwrap_or(id);
            let langs: Vec<String> = lang
                .map(|l| l.split(',').map(|x| x.trim().to_string()).collect())
                .unwrap_or_default();
            let c = sources::client();
            let list = sources::mangadex::chapters(&c, &id, &langs)
                .await
                .map_err(|e| anyhow!(e))?;
            if json || output::is_json_mode() {
                output::print_json(&list);
            } else {
                for ch in &list {
                    println!(
                        "{:>8}  {:<6} {}  {}",
                        ch.number.map(|n| n.to_string()).unwrap_or_default(),
                        ch.language,
                        ch.title,
                        ch.scanlator
                    );
                }
                println!("{} chapters", list.len());
            }
        }
        ReadCommand::Hash { file } => {
            let h = kosync::partial_md5_path(&file)
                .ok_or_else(|| anyhow!("can't read {}", file.display()))?;
            println!("{h}");
        }
    }
    Ok(())
}
