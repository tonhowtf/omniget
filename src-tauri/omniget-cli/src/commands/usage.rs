use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use chrono::{Local, TimeZone};
use omniget_core::core::llm::cli_runtime::accounts::{AccountStore, CliAccount, CliKind};
use omniget_core::core::llm::cli_usage::report::{report, CliUsageReport, ReportOptions};
use omniget_core::core::llm::cli_usage::windows::{UsageWindow, WindowKind, WindowSource};

fn report_key(a: &CliAccount) -> String {
    if a.config_dir.as_os_str().is_empty() {
        "default".to_string()
    } else {
        a.id.clone()
    }
}

pub fn bar(fraction: f32, width: usize) -> String {
    let filled = ((fraction.clamp(0.0, 1.0) * width as f32).round() as usize).min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

pub fn until(now_ms: i64, at_ms: i64) -> String {
    let mins = (at_ms - now_ms) / 60_000;
    if mins <= 0 {
        "agora".to_string()
    } else if mins < 60 {
        format!("em {mins} min")
    } else if mins < 48 * 60 {
        format!("em {}h{:02}", mins / 60, mins % 60)
    } else {
        format!("em {} dias", mins / (24 * 60))
    }
}

fn clock(at_ms: i64, now_ms: i64) -> String {
    let Some(t) = Local.timestamp_millis_opt(at_ms).single() else {
        return "?".into();
    };
    let today = Local
        .timestamp_millis_opt(now_ms)
        .single()
        .map(|n| n.date_naive());
    if Some(t.date_naive()) == today {
        t.format("%H:%M").to_string()
    } else {
        t.format("%d/%m %H:%M").to_string()
    }
}

fn tokens(n: u64) -> String {
    match n {
        n if n >= 1_000_000 => format!("{:.1} M", n as f64 / 1e6),
        n if n >= 1_000 => format!("{:.0} k", n as f64 / 1e3),
        n => n.to_string(),
    }
}

fn window_line(w: &UsageWindow, now_ms: i64) -> String {
    let name = match w.window {
        WindowKind::FiveHours => "5 h   ",
        WindowKind::SevenDays => "7 dias",
    };
    let src = match w.source {
        WindowSource::Real => "informado pelo CLI",
        WindowSource::Estimated => "estimado do histórico",
    };
    let reset = format!(
        "reseta {} ({})",
        clock(w.resets_at, now_ms),
        until(now_ms, w.resets_at)
    );
    match w.used_fraction {
        Some(f) => format!(
            "  {name} {} {:>3.0}%  {reset}  [{src}]",
            bar(f, 20),
            f * 100.0
        ),
        None => format!(
            "  {name} {} tokens usados, teto do plano desconhecido  {reset}  [{src}]",
            tokens(w.used_tokens)
        ),
    }
}

struct Spend {
    today: f64,
    period: f64,
    sessions: u32,
}

fn spend_by_account(r: &CliUsageReport, now_ms: i64) -> BTreeMap<String, Spend> {
    let midnight = Local
        .timestamp_millis_opt(now_ms)
        .single()
        .and_then(|n| n.date_naive().and_hms_opt(0, 0, 0))
        .and_then(|d| Local.from_local_datetime(&d).single())
        .map(|d| d.timestamp_millis())
        .unwrap_or(now_ms);
    let mut out: BTreeMap<String, Spend> = BTreeMap::new();
    for s in &r.sessions {
        let e = out.entry(s.account.clone()).or_insert(Spend {
            today: 0.0,
            period: 0.0,
            sessions: 0,
        });
        e.period += s.cost_usd;
        e.sessions += 1;
        if s.ended_at >= midnight {
            e.today += s.cost_usd;
        }
    }
    out
}

async fn build(days: u32) -> (Vec<CliAccount>, CliUsageReport) {
    let accounts: Vec<CliAccount> = AccountStore::default_store()
        .map(|s| s.list().iter().filter(|a| !a.disabled).cloned().collect())
        .unwrap_or_default();
    let roots: Vec<(String, PathBuf)> = accounts
        .iter()
        .filter(|a| !a.config_dir.as_os_str().is_empty())
        .map(|a| (a.id.clone(), a.config_dir.clone()))
        .collect();
    let opts = ReportOptions {
        days,
        tz_offset_minutes: Local::now().offset().local_minus_utc() / 60,
        ..Default::default()
    };
    let r = report(&roots, &opts).await;
    (accounts, r)
}

fn render(accounts: &[CliAccount], r: &CliUsageReport, days: u32) -> String {
    let now = r.generated_at;
    let claude: Vec<CliAccount> = accounts
        .iter()
        .filter(|a| a.cli == CliKind::Claude)
        .cloned()
        .collect();
    let who = super::claude::identities(&claude);
    let spend = spend_by_account(r, now);
    let mut out = format!(
        "Uso dos agentes · últimos {days} dias · {}\n",
        clock(now, now)
    );
    let mut shown = std::collections::HashSet::new();
    for a in accounts {
        let key = report_key(a);
        shown.insert(key.clone());
        let ident = claude
            .iter()
            .position(|c| c.id == a.id)
            .and_then(|i| who[i].clone())
            .filter(|i| i.logged_in)
            .map(|i| {
                format!(
                    " — {} · {}",
                    i.email.unwrap_or_default(),
                    i.plan.unwrap_or_default()
                )
            })
            .unwrap_or_default();
        out.push_str(&format!(
            "\n{} ({}, {}){ident}\n",
            a.label,
            a.id,
            a.cli.as_str()
        ));
        let mut ws: Vec<&UsageWindow> = r.windows.iter().filter(|w| w.account == key).collect();
        ws.sort_by_key(|w| w.window.minutes());
        if ws.is_empty() {
            out.push_str("  sem uso registrado nesse período\n");
        }
        for w in ws {
            out.push_str(&window_line(w, now));
            out.push('\n');
        }
        if let Some(s) = spend.get(&key) {
            out.push_str(&format!(
                "  gasto equivalente em API: hoje US$ {:.2} · {days} dias US$ {:.2} · {} sessões\n",
                s.today, s.period, s.sessions
            ));
        }
    }
    for (key, s) in &spend {
        if !shown.contains(key) {
            out.push_str(&format!(
                "\n{key} (sem conta no app)\n  gasto equivalente em API: hoje US$ {:.2} · {days} dias US$ {:.2} · {} sessões\n",
                s.today, s.period, s.sessions
            ));
        }
    }
    out.push_str(&format!(
        "\nTotal: US$ {:.2} em {} dias · {} entrada / {} saída / {} cache\n",
        r.totals.cost_usd,
        days,
        tokens(r.totals.input_tokens),
        tokens(r.totals.output_tokens),
        tokens(r.totals.cache_read_tokens + r.totals.cache_write_tokens)
    ));
    out
}

pub async fn execute(days: u32, watch: Option<u64>, json: bool) -> anyhow::Result<()> {
    let days = days.clamp(1, 365);
    loop {
        let (accounts, r) = build(days).await;
        if json {
            println!("{}", serde_json::to_string_pretty(&r)?);
        } else {
            let text = render(&accounts, &r, days);
            if watch.is_some() {
                print!("\x1b[2J\x1b[H");
            }
            print!("{text}");
            std::io::stdout().flush()?;
        }
        match watch {
            Some(secs) if !json => {
                println!("\natualiza a cada {secs} s · Ctrl+C para sair");
                tokio::time::sleep(std::time::Duration::from_secs(secs.max(5))).await;
            }
            _ => return Ok(()),
        }
    }
}
