use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub start: i64,
    pub end: i64,
    pub soft: bool,
}

const DAY: i64 = 86_400_000;

fn day_start(d: NaiveDate) -> i64 {
    d.and_hms_opt(0, 0, 0)
        .and_then(|n| Local.from_local_datetime(&n).earliest())
        .map(|d| d.timestamp_millis())
        .unwrap_or_default()
}

fn days(from: NaiveDate, to_exclusive: NaiveDate) -> Window {
    Window {
        start: day_start(from),
        end: day_start(to_exclusive),
        soft: false,
    }
}

fn month_window(year: i32, month: u32) -> Option<Window> {
    let from = NaiveDate::from_ymd_opt(year, month, 1)?;
    let to = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)?
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)?
    };
    Some(days(from, to))
}

fn year_window(year: i32) -> Option<Window> {
    Some(days(
        NaiveDate::from_ymd_opt(year, 1, 1)?,
        NaiveDate::from_ymd_opt(year + 1, 1, 1)?,
    ))
}

fn month_of(word: &str) -> Option<u32> {
    const NAMES: &[(&str, u32)] = &[
        ("janeiro", 1),
        ("january", 1),
        ("enero", 1),
        ("fevereiro", 2),
        ("february", 2),
        ("febrero", 2),
        ("marco", 3),
        ("march", 3),
        ("marzo", 3),
        ("abril", 4),
        ("april", 4),
        ("maio", 5),
        ("may", 5),
        ("mayo", 5),
        ("junho", 6),
        ("june", 6),
        ("junio", 6),
        ("julho", 7),
        ("july", 7),
        ("julio", 7),
        ("agosto", 8),
        ("august", 8),
        ("setembro", 9),
        ("september", 9),
        ("septiembre", 9),
        ("outubro", 10),
        ("october", 10),
        ("octubre", 10),
        ("novembro", 11),
        ("november", 11),
        ("noviembre", 11),
        ("dezembro", 12),
        ("december", 12),
        ("diciembre", 12),
    ];
    NAMES.iter().find(|(n, _)| *n == word).map(|(_, m)| *m)
}

fn unit_days(word: &str) -> Option<i64> {
    let w = word.trim_end_matches('s');
    Some(match w {
        "dia" | "day" => 1,
        "semana" | "week" => 7,
        "me" | "mese" | "month" | "mes" => 30,
        "ano" | "year" | "ano_" => 365,
        _ => return None,
    })
}

fn number(word: &str) -> Option<i64> {
    if let Ok(n) = word.parse::<i64>() {
        return (1..=500).contains(&n).then_some(n);
    }
    Some(match word {
        "um" | "uma" | "one" | "a" | "an" | "un" | "una" => 1,
        "dois" | "duas" | "two" | "dos" => 2,
        "tres" | "three" => 3,
        "quatro" | "four" | "cuatro" => 4,
        "cinco" | "five" => 5,
        "seis" | "six" => 6,
        "sete" | "seven" | "siete" => 7,
        "dez" | "ten" | "diez" => 10,
        _ => return None,
    })
}

pub fn parse(query: &str, now: i64) -> Option<Window> {
    let today = Local
        .timestamp_millis_opt(now)
        .single()
        .map(|d| d.date_naive())?;
    let text = super::graph::fold(query);
    let w: Vec<&str> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '/'))
        .filter(|s| !s.is_empty())
        .collect();
    let has = |s: &str| w.contains(&s);
    let pair = |a: &str, b: &str| w.windows(2).any(|p| p[0] == a && p[1] == b);

    for t in &w {
        if let Ok(d) = NaiveDate::parse_from_str(t, "%Y-%m-%d") {
            return Some(days(d, d + Duration::days(1)));
        }
        if let Ok(d) = NaiveDate::parse_from_str(t, "%d/%m/%Y") {
            return Some(days(d, d + Duration::days(1)));
        }
    }

    if has("anteontem") {
        let d = today - Duration::days(2);
        return Some(days(d, d + Duration::days(1)));
    }
    if has("ontem") || has("yesterday") || has("ayer") {
        let d = today - Duration::days(1);
        return Some(days(d, d + Duration::days(1)));
    }
    if has("hoje") || has("today") || has("hoy") || pair("esta", "manha") || pair("this", "morning")
    {
        return Some(days(today, today + Duration::days(1)));
    }

    for (i, t) in w.iter().enumerate() {
        let ago_before = i > 0 && matches!(w[i - 1], "ha" | "hace" | "faz");
        if let (Some(n), Some(u)) = (number(t), w.get(i + 1).and_then(|u| unit_days(u))) {
            let ago_after = w
                .get(i + 2)
                .map(|x| *x == "ago" || *x == "atras")
                .unwrap_or(false);
            let span = n * u;
            if ago_before || ago_after {
                let center = today - Duration::days(span);
                let slack = (u / 2).max(1);
                return Some(days(
                    center - Duration::days(slack),
                    center + Duration::days(slack + 1),
                ));
            }
            let last = i > 0
                && matches!(
                    w[i - 1],
                    "ultimos" | "ultimas" | "last" | "past" | "ultimo" | "ultima"
                );
            if last {
                return Some(days(
                    today - Duration::days(span),
                    today + Duration::days(1),
                ));
            }
        }
    }

    let week_start = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    if pair("semana", "passada") || pair("last", "week") || pair("semana", "pasada") {
        return Some(days(week_start - Duration::days(7), week_start));
    }
    if pair("esta", "semana") || pair("this", "week") || pair("essa", "semana") {
        return Some(days(week_start, today + Duration::days(1)));
    }
    if pair("mes", "passado") || pair("last", "month") || pair("mes", "pasado") {
        let (y, m) = if today.month() == 1 {
            (today.year() - 1, 12)
        } else {
            (today.year(), today.month() - 1)
        };
        return month_window(y, m);
    }
    if pair("este", "mes") || pair("this", "month") || pair("esse", "mes") {
        return month_window(today.year(), today.month());
    }
    if pair("ano", "passado") || pair("last", "year") || pair("ano", "pasado") {
        return year_window(today.year() - 1);
    }
    if pair("este", "ano") || pair("this", "year") || pair("esse", "ano") {
        return year_window(today.year());
    }

    for (i, t) in w.iter().enumerate() {
        let Some(m) = month_of(t) else { continue };
        let year = w
            .iter()
            .skip(i + 1)
            .take(2)
            .find_map(|y| y.parse::<i32>().ok().filter(|y| (1900..=2200).contains(y)));
        let lead = i > 0 && matches!(w[i - 1], "em" | "in" | "en" | "de" | "no" | "during");
        if year.is_none() && !lead && (*t == "may" || *t == "marco") {
            continue;
        }
        let y = year.unwrap_or_else(|| {
            if m > today.month() {
                today.year() - 1
            } else {
                today.year()
            }
        });
        return month_window(y, m);
    }
    for (i, t) in w.iter().enumerate() {
        if let Ok(y) = t.parse::<i32>() {
            let lead = i > 0 && matches!(w[i - 1], "em" | "in" | "en" | "de" | "during");
            if (1900..=2200).contains(&y) && lead {
                return year_window(y);
            }
        }
    }

    if has("recentemente")
        || has("recently")
        || has("lately")
        || has("ultimamente")
        || has("recientemente")
    {
        return Some(Window {
            start: now - 30 * DAY,
            end: now + DAY,
            soft: true,
        });
    }
    None
}

pub fn proximity(win: &Window, s: i64, e: i64) -> f32 {
    if e >= win.start && s < win.end {
        return 1.0;
    }
    let gap = if e < win.start {
        win.start - e
    } else {
        s - win.end
    };
    let scale = (win.end - win.start).max(7 * DAY) as f32;
    (1.0 - gap as f32 / scale).max(0.0)
}
