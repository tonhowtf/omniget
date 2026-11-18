use std::time::Duration;

use serde_json::{json, Value};

use super::args::{opt_str, req_str};
use super::model::{page, s, Platform};
use super::page as web;
use super::shortpost::{base_headers, gql, tweet_id};

const P: Platform = Platform::ShortPost;
const PATH: &str = "/2/grok/add_response.json";

const SEND: &str = r#"(async()=>{
const a=__ARGS__;
let sign=null;
try{
const k=Object.keys(window).find(k=>k.startsWith('webpackChunk'));
let req=null;
window[k].push([[Symbol('omniget')],{},r=>{req=r}]);
for(const ch of window[k]){const mods=ch&&ch[1];if(!mods)continue;for(const id in mods){try{const t=String(mods[id]);if(t.includes('rweb_client_transaction_id_enabled')&&t.includes('x-client-transaction-id')){const f=Object.values(req(id)).find(f=>typeof f==='function'&&f.length===3);if(f)sign=f;}}catch(e){}}}
}catch(e){}
const h=a.headers;
h['content-type']='text/plain;charset=UTF-8';
h['x-xai-request-id']=crypto.randomUUID();
if(sign){try{h['x-client-transaction-id']=await sign('https://grok.x.com',a.path,'POST');}catch(e){}}
try{
const r=await fetch('https://grok.x.com'+a.path,{method:'POST',headers:h,credentials:'include',body:a.body});
return {status:r.status,body:await r.text(),signed:!!sign};
}catch(e){return {status:0,body:String(e),signed:!!sign};}
})()"#;

fn conversation_id(input: &str) -> Option<String> {
    let t = input.trim();
    if !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit()) {
        return Some(t.to_string());
    }
    let re = regex::Regex::new(r"[?&]conversation=(\d+)").expect("static regex");
    re.captures(t).map(|c| c[1].to_string())
}

fn chat_url(id: &str) -> String {
    format!("https://x.com/i/grok?conversation={id}")
}

fn card(raw: &Value) -> Option<(String, String)> {
    let v: Value = serde_json::from_str(raw.as_str()?).ok()?;
    let id = v["id"].as_str()?.to_string();
    let url = v["url"].as_str()?.to_string();
    (!id.is_empty() && !url.is_empty()).then_some((id, url))
}

fn clean(text: &str, cards: &[(String, String)]) -> (String, Vec<String>) {
    let re =
        regex::Regex::new(r#"(?s)<grok:render\b([^>]*)>.*?</grok:render>"#).expect("static regex");
    let id_re = regex::Regex::new(r#"card_id="([^"]+)""#).expect("static regex");
    let mut sources: Vec<String> = Vec::new();
    let out = re.replace_all(text, |c: &regex::Captures| {
        let url = id_re
            .captures(&c[1])
            .and_then(|m| cards.iter().find(|(id, _)| *id == m[1]))
            .map(|(_, url)| url.clone());
        match url {
            Some(url) => {
                let n = match sources.iter().position(|u| *u == url) {
                    Some(i) => i + 1,
                    None => {
                        sources.push(url);
                        sources.len()
                    }
                };
                format!(" [{n}]")
            }
            None => String::new(),
        }
    });
    (out.trim().to_string(), sources)
}

pub fn reply(body: &str) -> Result<Value, String> {
    let mut text = String::new();
    let mut cards = Vec::new();
    let mut follow_ups = Vec::new();
    let mut conversation = String::new();
    let mut notice = String::new();
    for line in body.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(e) = v["errors"].as_array().and_then(|a| a.first()) {
            return Err(format!(
                "REJECTED: the built-in assistant refused the request: {} (code {})",
                s(&e["message"]),
                s(&e["code"])
            ));
        }
        if let Some(id) = v["conversationId"].as_str() {
            conversation = id.to_string();
        }
        let r = &v["result"];
        if let Some(c) = card(&r["cardAttachment"]) {
            cards.push(c);
        }
        if let Some(list) = r["followUpSuggestions"].as_array() {
            follow_ups.extend(
                list.iter()
                    .filter_map(|f| f["label"].as_str().map(String::from)),
            );
        }
        let Some(m) = r["message"].as_str() else {
            continue;
        };
        if r["messageTag"] == "final" && r["isThinking"] != true {
            text.push_str(m);
        } else if r["messageTag"].is_null() && notice.is_empty() {
            notice = m.to_string();
        }
    }
    let (text, sources) = clean(&text, &cards);
    if text.is_empty() {
        let why: String = if notice.is_empty() { body } else { &notice }
            .chars()
            .take(300)
            .collect();
        return Err(format!(
            "REJECTED: the built-in assistant did not answer: {}",
            if why.trim().is_empty() {
                "empty reply"
            } else {
                why.trim()
            }
        ));
    }
    Ok(
        json!({ "conversation_id": conversation, "text": text, "sources": sources, "follow_ups": follow_ups }),
    )
}

fn message(item: &Value) -> Value {
    let cards: Vec<(String, String)> = item["card_attachments"]
        .as_array()
        .map(|a| a.iter().filter_map(card).collect())
        .unwrap_or_default();
    let (text, sources) = clean(&s(&item["message"]), &cards);
    json!({
        "id": s(&item["chat_item_id"]),
        "role": if item["sender_type"] == "User" { "user" } else { "assistant" },
        "text": text,
        "sources": sources,
        "created_at": item["created_at_ms"].as_i64().map(|ms| ms / 1000),
    })
}

async fn ask(a: &Value) -> Result<Value, String> {
    let mut text = req_str(a, "message")?;
    if let Some(p) = opt_str(a, "post") {
        let id = tweet_id(&p).ok_or("INVALID_ARGUMENTS: post must be a post id or link")?;
        text = format!("{text}\n\nhttps://x.com/i/status/{id}");
    }
    let conversation = match opt_str(a, "conversation") {
        Some(c) => conversation_id(&c).ok_or(
            "INVALID_ARGUMENTS: conversation must be an assistant conversation id or an x.com/i/grok?conversation=... link",
        )?,
        None => {
            let v = gql("CreateGrokConversation", json!({}), true).await?;
            let id = s(&v["data"]["create_grok_conversation"]["conversation_id"]);
            if id.is_empty() {
                return Err("UPSTREAM_CHANGED: Short Posts did not open an assistant conversation".into());
            }
            id
        }
    };
    let body = json!({
        "responses": [{ "message": text, "sender": 1, "promptSource": "", "fileAttachments": [] }],
        "systemPromptName": "",
        "conversationId": conversation,
        "returnSearchResults": true,
        "returnCitations": true,
        "promptMetadata": { "promptSource": "NATURAL", "action": "INPUT" },
        "imageGenerationCount": 4,
        "requestFeatures": { "eagerTweets": true, "serverHistory": true },
    });
    let args = json!({ "headers": base_headers().await?, "path": PATH, "body": body.to_string() });
    let b = web::ready(P).await?;
    let r = web::eval_for(
        &b,
        P,
        &SEND.replace("__ARGS__", &args.to_string()),
        Duration::from_secs(300),
    )
    .await?;
    let raw = s(&r["body"]);
    match r["status"].as_u64().unwrap_or(0) {
        200..=299 => {}
        0 => return Err("NETWORK: could not reach the built-in assistant".into()),
        401 => return Err("NOT_LOGGED_IN: Short Posts rejected the session (401); sign in again".into()),
        429 => {
            return Err(
                "RATE_LIMITED: the built-in assistant is limiting this account; wait before asking again".into(),
            )
        }
        404 if r["signed"] != true => {
            return Err("UPSTREAM_CHANGED: could not sign the assistant request; reload x.com in the OmniGet browser and try again".into())
        }
        code => {
            let head: String = raw.chars().take(300).collect();
            return Err(format!("UPSTREAM_{code}: the built-in assistant answered {code}: {}", head.trim()));
        }
    }
    let mut out = reply(&raw)?;
    out["conversation_id"] = json!(conversation);
    out["url"] = json!(chat_url(&conversation));
    Ok(out)
}

pub async fn call(tool: &str, a: &Value) -> Result<Value, String> {
    match tool {
        "x_assistant" => ask(a).await,
        "x_assistant_chats" => {
            let mut vars = json!({});
            if let Some(c) = opt_str(a, "cursor") {
                vars["cursor"] = json!(c);
            }
            let v = gql("GrokHistory", vars, false).await?;
            let h = &v["data"]["grok_conversation_history"];
            let items: Vec<Value> = h["items"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter_map(|i| {
                            let id = i["grokConversation"]["rest_id"].as_str()?;
                            Some(json!({
                                "id": id,
                                "title": s(&i["title"]),
                                "created_at": i["created_at_ms"].as_i64().map(|ms| ms / 1000),
                                "url": chat_url(id),
                            }))
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(page(items, h["cursor"].as_str().map(String::from)))
        }
        "x_assistant_chat" => {
            let id = conversation_id(&req_str(a, "conversation")?).ok_or(
                "INVALID_ARGUMENTS: conversation must be an assistant conversation id or an x.com/i/grok?conversation=... link",
            )?;
            let mut vars = json!({ "restId": id });
            if let Some(c) = opt_str(a, "cursor") {
                vars["cursor"] = json!(c);
            }
            let v = gql("GrokConversationItemsByRestId", vars, false).await?;
            let d = &v["data"]["grok_conversation_items_by_rest_id"];
            let mut items: Vec<Value> = d["items"]
                .as_array()
                .map(|list| list.iter().map(message).collect())
                .unwrap_or_default();
            items.reverse();
            let cursor = d["cursor"]
                .as_str()
                .map(String::from)
                .filter(|_| !items.is_empty());
            let mut out = page(items, cursor);
            out["conversation_id"] = json!(id);
            out["url"] = json!(chat_url(&id));
            Ok(out)
        }
        _ => Err(format!("UNKNOWN_TOOL: {tool}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_the_stream_and_numbers_the_citations() {
        let body = [
            r#"{"conversationId":"77","userChatItemId":"1","agentChatItemId":"2"}"#,
            r#"{"result":{"sender":"ASSISTANT","message":"Thinking about your request","isThinking":true,"messageTag":"header"}}"#,
            r#"{"result":{"sender":"assistant","message":"<xai:tool_usage_card>x</xai:tool_usage_card>","isThinking":true,"messageTag":"tool_usage_card"}}"#,
            r#"{"result":{"sender":"ASSISTANT","message":"Latest post","messageTag":"final"}}"#,
            r#"{"result":{"sender":"assistant","message":"<grok:render card_id=\"ab\" card_type=\"citation_card\" type=\"render_inline_citation\"><argument name=\"citation_id\">0</argument></grok:render>","cardAttachment":"{\"id\":\"ab\",\"url\":\"https://x.com/a/status/1\"}","messageTag":"final"}}"#,
            r#"{"result":{"sender":"ASSISTANT","message":" and more<grok:render card_id=\"zz\"></grok:render>.","messageTag":"final"}}"#,
            r#"{"result":{"responseChatItemId":"2","followUpSuggestions":[{"label":"Go deeper"}]}}"#,
        ]
        .join("\n");
        let r = reply(&body).unwrap();
        assert_eq!(r["text"], "Latest post [1] and more.");
        assert_eq!(r["sources"], json!(["https://x.com/a/status/1"]));
        assert_eq!(r["follow_ups"], json!(["Go deeper"]));
        assert_eq!(r["conversation_id"], "77");
    }

    #[test]
    fn reports_a_refusal_and_an_empty_reply() {
        let e = reply(r#"{"errors":[{"message":"Sorry, that page does not exist","code":34}]}"#)
            .unwrap_err();
        assert!(
            e.starts_with("REJECTED: the built-in assistant refused"),
            "{e}"
        );
        let e = reply(r#"{"result":{"message":"You reached your limit"}}"#).unwrap_err();
        assert_eq!(
            e,
            "REJECTED: the built-in assistant did not answer: You reached your limit"
        );
    }

    #[test]
    fn reads_conversation_ids_and_stored_messages() {
        assert_eq!(conversation_id(" 42 ").as_deref(), Some("42"));
        assert_eq!(
            conversation_id("https://x.com/i/grok?conversation=99&x=1").as_deref(),
            Some("99")
        );
        assert_eq!(conversation_id("abc"), None);
        let m = message(
            &json!({ "chat_item_id": "5", "sender_type": "Agent", "created_at_ms": 2000,
            "message": "See<grok:render card_id=\"c\"></grok:render>",
            "card_attachments": ["{\"id\":\"c\",\"url\":\"https://e.com\"}"] }),
        );
        assert_eq!(m["role"], "assistant");
        assert_eq!(m["text"], "See [1]");
        assert_eq!(m["sources"], json!(["https://e.com"]));
        assert_eq!(m["created_at"], 2);
    }
}
