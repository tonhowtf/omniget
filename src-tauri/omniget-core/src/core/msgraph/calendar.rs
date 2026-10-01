use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct CalendarMeeting {
    pub id: String,
    pub subject: String,
    pub start: String,
    pub end: String,
    pub organizer: String,
    pub attendees: Vec<String>,
    pub join_url: Option<String>,
    pub web_link: Option<String>,
    pub is_organizer: bool,
}

fn utc(v: &Value) -> String {
    let t = v["dateTime"].as_str().unwrap_or("");
    let t = t.split('.').next().unwrap_or(t);
    if t.is_empty() || t.ends_with('Z') {
        t.to_string()
    } else {
        format!("{t}Z")
    }
}

fn event(v: &Value) -> CalendarMeeting {
    let join_url = v["onlineMeeting"]["joinUrl"]
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            v["location"]["displayName"]
                .as_str()
                .filter(|l| l.starts_with("https://teams.microsoft.com/"))
                .map(str::to_string)
        });
    CalendarMeeting {
        id: v["id"].as_str().unwrap_or("").to_string(),
        subject: v["subject"].as_str().unwrap_or("(no subject)").to_string(),
        start: utc(&v["start"]),
        end: utc(&v["end"]),
        organizer: v["organizer"]["emailAddress"]["name"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        attendees: v["attendees"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| a["emailAddress"]["name"].as_str().map(str::to_string))
            .collect(),
        join_url,
        web_link: v["webLink"].as_str().map(str::to_string),
        is_organizer: v["isOrganizer"].as_bool().unwrap_or(false),
    }
}

pub async fn meetings(
    past_days: i64,
    days: i64,
    only_online: bool,
) -> anyhow::Result<Vec<CalendarMeeting>> {
    let now = chrono::Utc::now();
    let from = (now - chrono::Duration::days(past_days.max(0))).format("%Y-%m-%dT%H:%M:%SZ");
    let to = (now + chrono::Duration::days(days.max(0))).format("%Y-%m-%dT%H:%M:%SZ");
    let path = format!(
        "/me/calendarView?startDateTime={from}&endDateTime={to}&$top=50&$orderby=start/dateTime\
&$select=subject,start,end,isOnlineMeeting,onlineMeeting,organizer,attendees,webLink,location,isOrganizer,isCancelled"
    );
    let raw = super::get_paged(&path, 500, |_| false).await?;
    Ok(raw
        .iter()
        .filter(|v| v["isCancelled"].as_bool() != Some(true))
        .map(event)
        .filter(|m| !only_online || m.join_url.is_some())
        .collect())
}
