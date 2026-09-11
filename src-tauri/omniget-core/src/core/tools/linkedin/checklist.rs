//! Score offline de completude do perfil. Cada item olha um campo real do
//! export (`Profile.csv`, `Positions.csv`, `Education.csv`, `Skills.csv`,
//! `Endorsement_Received_Info.csv`, `Recommendations_Received.csv`) e diz o
//! numero que viu contra o alvo. Nada de conselho generico: item sem fonte no
//! export sai como "warn" e nao entra na conta.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::csv;
use super::source::Source;
use crate::core::tools::{report, ProgressFn};

const ID: &str = "li-checklist";

/// Alvos objetivos. Ficam aqui para a UI poder explicar o porque.
const HEADLINE_MIN: usize = 40;
const ABOUT_MIN: usize = 200;
const ROLE_DESC_MIN: usize = 150;
const SKILLS_MIN: usize = 5;
const POSITIONS_MIN: usize = 2;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Options {
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckItem {
    /// Casa com `tools.licheck.item_<id>` na UI.
    pub id: String,
    /// "ok" | "miss" | "warn" (sem fonte no export, fora da conta).
    pub state: String,
    pub weight: u32,
    /// O que o export trouxe (numero ou texto curto).
    pub value: String,
    /// O alvo do item, quando ha um.
    pub target: String,
    /// Arquivo e coluna de onde veio, para o item ser conferivel.
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChecklistResult {
    pub path: String,
    pub name: String,
    pub headline: String,
    pub score: u32,
    pub max: u32,
    pub percent: u32,
    pub items: Vec<CheckItem>,
    pub missing: Vec<String>,
    pub unknown: Vec<String>,
}

fn item(id: &str, ok: bool, weight: u32, value: String, target: &str, source: &str) -> CheckItem {
    CheckItem {
        id: id.to_string(),
        state: if ok { "ok" } else { "miss" }.to_string(),
        weight,
        value,
        target: target.to_string(),
        source: source.to_string(),
    }
}

fn unknown(id: &str, weight: u32, source: &str) -> CheckItem {
    CheckItem {
        id: id.to_string(),
        state: "warn".to_string(),
        weight,
        value: String::new(),
        target: String::new(),
        source: source.to_string(),
    }
}

fn is_image(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [".jpg", ".jpeg", ".png", ".webp", ".gif"]
        .iter()
        .any(|e| n.ends_with(e))
}

/// URL vaidosa: sem o sufixo aleatorio que o LinkedIn coloca por padrao
/// (`ana-silva-1a2b3c4`, `anasilva123456789`).
pub fn looks_custom(slug: &str) -> bool {
    let s = slug.trim_matches('/').to_lowercase();
    if s.is_empty() {
        return false;
    }
    let tail = s.rsplit('-').next().unwrap_or(&s);
    if tail.len() >= 6 && tail.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    let digits = s.chars().rev().take_while(char::is_ascii_digit).count();
    digits < 6
}

/// Procura a URL do proprio perfil no `messages.csv` (a coluna
/// SENDER PROFILE URL das mensagens que voce mandou).
fn own_url(src: &Source, name: &str) -> String {
    if name.trim().is_empty() {
        return String::new();
    }
    let t = src.table_or_empty(&["messages.csv"]);
    for row in &t.rows {
        let from = t.get(row, &["FROM", "From"]);
        if from.trim().eq_ignore_ascii_case(name.trim()) {
            let url = t.get(row, &["SENDER PROFILE URL", "SenderProfileUrl"]);
            if !url.trim().is_empty() {
                return url.trim().to_string();
            }
        }
    }
    String::new()
}

pub fn run(opts: &Options, p: &ProgressFn) -> Result<ChecklistResult> {
    report(p, ID, "started", 0, Some(2), None);
    let opened = Source::open(&opts.path)?;
    let src = &opened.source;

    let profile = src.table_or_empty(&["Profile.csv"]);
    let empty_row: Vec<String> = Vec::new();
    let prow = profile.rows.first().unwrap_or(&empty_row);
    let field = |names: &[&str]| profile.get(prow, names).to_string();

    let first = field(&["First Name"]);
    let last = field(&["Last Name"]);
    let name = format!("{} {}", first, last).trim().to_string();
    let headline = field(&["Headline"]);
    let about = field(&["Summary", "About"]);

    let positions = src.table_or_empty(&["Positions.csv"]);
    let current = positions.rows.iter().find(|r| {
        positions
            .get(r, &["Finished On", "End Date", "FinishedOn"])
            .is_empty()
    });
    let role_desc = current
        .map(|r| positions.get(r, &["Description"]).to_string())
        .unwrap_or_default();

    let education = src.table_or_empty(&["Education.csv"]);
    let skills = src.table_or_empty(&["Skills.csv"]);
    let endorsements = src.table_or_empty(&["Endorsement_Received_Info.csv", "Endorsements.csv"]);
    let recs = src.table_or_empty(&["Recommendations_Received.csv"]);
    let visible_recs = if recs.col(&["Status"]).is_some() {
        recs.rows
            .iter()
            .filter(|r| recs.get(r, &["Status"]).eq_ignore_ascii_case("VISIBLE"))
            .count()
    } else {
        recs.len()
    };
    report(p, ID, "progress", 1, Some(2), None);

    let mut items = vec![
        item(
            "headline",
            headline.chars().count() >= HEADLINE_MIN,
            10,
            headline.chars().count().to_string(),
            &HEADLINE_MIN.to_string(),
            "Profile.csv · Headline",
        ),
        item(
            "about",
            about.chars().count() >= ABOUT_MIN,
            15,
            about.chars().count().to_string(),
            &ABOUT_MIN.to_string(),
            "Profile.csv · Summary",
        ),
        item(
            "industry",
            !field(&["Industry"]).is_empty(),
            5,
            field(&["Industry"]),
            "",
            "Profile.csv · Industry",
        ),
        item(
            "location",
            !field(&["Geo Location", "Location"]).is_empty(),
            5,
            field(&["Geo Location", "Location"]),
            "",
            "Profile.csv · Geo Location",
        ),
        item(
            "website",
            !field(&["Websites", "Website"]).is_empty(),
            5,
            field(&["Websites", "Website"]),
            "",
            "Profile.csv · Websites",
        ),
    ];

    // Foto: so da para responder se o export veio com as imagens.
    let photo = opened
        .entries
        .iter()
        .find(|e| is_image(e) && e.to_lowercase().contains("photo"));
    let has_media = opened.entries.iter().any(|e| is_image(e));
    items.push(match (photo, has_media) {
        (Some(f), _) => item(
            "photo",
            true,
            5,
            f.rsplit('/').next().unwrap_or(f).to_string(),
            "",
            "Profile Photos/",
        ),
        (None, true) => item("photo", false, 5, String::new(), "", "Profile Photos/"),
        (None, false) => unknown("photo", 5, "Profile Photos/"),
    });

    items.push(item(
        "current_role",
        current.is_some(),
        10,
        current
            .map(|r| positions.get(r, &["Title"]).to_string())
            .unwrap_or_default(),
        "",
        "Positions.csv · Finished On vazio",
    ));
    items.push(item(
        "role_description",
        role_desc.chars().count() >= ROLE_DESC_MIN,
        10,
        role_desc.chars().count().to_string(),
        &ROLE_DESC_MIN.to_string(),
        "Positions.csv · Description",
    ));
    items.push(item(
        "positions",
        positions.len() >= POSITIONS_MIN,
        5,
        positions.len().to_string(),
        &POSITIONS_MIN.to_string(),
        "Positions.csv",
    ));
    items.push(item(
        "education",
        !education.is_empty(),
        10,
        education.len().to_string(),
        "1",
        "Education.csv",
    ));
    items.push(item(
        "skills",
        skills.len() >= SKILLS_MIN,
        10,
        skills.len().to_string(),
        &SKILLS_MIN.to_string(),
        "Skills.csv",
    ));
    items.push(item(
        "endorsements",
        !endorsements.is_empty(),
        5,
        endorsements.len().to_string(),
        "1",
        "Endorsement_Received_Info.csv",
    ));
    items.push(item(
        "recommendations",
        visible_recs > 0,
        10,
        visible_recs.to_string(),
        "1",
        "Recommendations_Received.csv · Status VISIBLE",
    ));

    let url = own_url(src, &name);
    let slug = url
        .split(['?', '#'])
        .next()
        .unwrap_or(&url)
        .trim_end_matches('/')
        .rsplit("/in/")
        .next()
        .filter(|_| url.contains("/in/"))
        .unwrap_or("")
        .to_string();
    items.push(if slug.is_empty() {
        unknown("custom_url", 5, "messages.csv · SENDER PROFILE URL")
    } else {
        item(
            "custom_url",
            looks_custom(&slug),
            5,
            slug.clone(),
            "",
            "messages.csv · SENDER PROFILE URL",
        )
    });

    let max: u32 = items
        .iter()
        .filter(|i| i.state != "warn")
        .map(|i| i.weight)
        .sum();
    let score: u32 = items
        .iter()
        .filter(|i| i.state == "ok")
        .map(|i| i.weight)
        .sum();
    let out = ChecklistResult {
        path: opts.path.clone(),
        name,
        headline,
        score,
        max,
        percent: (score * 100).checked_div(max).unwrap_or(0),
        missing: items
            .iter()
            .filter(|i| i.state == "miss")
            .map(|i| i.id.clone())
            .collect(),
        unknown: items
            .iter()
            .filter(|i| i.state == "warn")
            .map(|i| i.id.clone())
            .collect(),
        items,
    };
    report(p, ID, "done", 2, Some(2), None);
    Ok(out)
}

/// Usado pelo painel geral para nao repetir a leitura do perfil.
pub fn slug_of(url: &str) -> String {
    csv::norm(url.rsplit("/in/").next().unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::super::source::fixture;
    use super::*;
    use crate::core::tools::noop_progress;

    fn run_dir(dir: &std::path::Path) -> ChecklistResult {
        run(
            &Options {
                path: dir.to_string_lossy().to_string(),
            },
            &noop_progress(),
        )
        .expect("roda")
    }

    #[test]
    fn pontua_o_perfil_sintetico() {
        let dir = fixture::dir();
        let r = run_dir(&dir);
        assert_eq!(r.name, "Tonho Dev");
        assert_eq!(r.items.len(), 14);
        // Sem imagens no export: foto fica fora da conta.
        assert!(r.unknown.contains(&"photo".to_string()));
        assert_eq!(r.max, 105);
        // Headline de 50 passa; about de 47, descricao de 24 e 3 skills nao.
        assert!(r.missing.contains(&"about".to_string()));
        assert!(r.missing.contains(&"role_description".to_string()));
        assert!(r.missing.contains(&"skills".to_string()));
        assert!(!r.missing.contains(&"education".to_string()));
        assert!(!r.missing.contains(&"recommendations".to_string()));
        assert_eq!(r.score, 70);
        assert_eq!(r.percent, 66);
    }

    #[test]
    fn cada_item_aponta_uma_fonte_do_export() {
        let dir = fixture::dir();
        let r = run_dir(&dir);
        assert!(r
            .items
            .iter()
            .all(|i| i.source.contains(".csv") || i.source.contains('/')));
        let h = r
            .items
            .iter()
            .find(|i| i.id == "headline")
            .expect("headline");
        assert_eq!(h.value, "50");
        assert_eq!(h.target, "40");
    }

    #[test]
    fn url_customizada_vem_do_messages() {
        let dir = fixture::dir();
        let r = run_dir(&dir);
        let u = r.items.iter().find(|i| i.id == "custom_url").expect("url");
        assert_eq!(u.state, "ok");
        assert_eq!(u.value, "tonhodev");
    }

    #[test]
    fn url_com_sufixo_aleatorio_nao_conta_como_custom() {
        assert!(looks_custom("tonho-dev"));
        assert!(looks_custom("anasilva"));
        assert!(!looks_custom("ana-silva-1a2b3c4"));
        assert!(!looks_custom("anasilva123456"));
        assert!(!looks_custom(""));
        assert_eq!(slug_of("https://www.linkedin.com/in/ana-silva"), "anasilva");
    }

    #[test]
    fn export_sem_perfil_nao_estoura() {
        let tmp = std::env::temp_dir().join(format!("omniget-li-chk-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let _ = std::fs::write(tmp.join("Skills.csv"), "Name\nRust\n");
        let r = run_dir(&tmp);
        assert_eq!(r.score, 0);
        assert!(r.percent < 100);
        assert!(r.missing.contains(&"headline".to_string()));
    }
}
