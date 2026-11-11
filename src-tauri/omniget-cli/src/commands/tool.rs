use anyhow::bail;
use serde_json::{json, Map, Value};

pub fn parse_input(args: &[String]) -> anyhow::Result<Value> {
    if let [only] = args {
        if only.trim_start().starts_with('{') {
            let value: Value = serde_json::from_str(only)?;
            return Ok(value);
        }
    }
    let mut out = Map::new();
    for arg in args {
        let Some((key, raw)) = arg.split_once('=') else {
            bail!("expected key=value or one JSON object, got `{arg}`");
        };
        let value = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()));
        out.insert(key.to_string(), value);
    }
    Ok(Value::Object(out))
}

pub async fn execute(
    name: Option<String>,
    args: Vec<String>,
    json_out: bool,
) -> anyhow::Result<()> {
    let bridge = super::launch::connect().await?;
    let Some(name) = name else {
        let listed = bridge.post("/v1/tool/list", json!({})).await?;
        if json_out {
            println!("{}", serde_json::to_string_pretty(&listed)?);
            return Ok(());
        }
        for tool in listed["tools"].as_array().into_iter().flatten() {
            let description = tool["description"].as_str().unwrap_or("");
            let first = description.split(". ").next().unwrap_or(description);
            println!("{:<26} {first}", tool["name"].as_str().unwrap_or(""));
        }
        return Ok(());
    };
    let out = bridge
        .post(&format!("/v1/tool/{name}"), parse_input(&args)?)
        .await?;
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn one_json_object_is_taken_as_the_whole_input() {
        let input = parse_input(&args(&[r#"{"op":"list","limit":5}"#])).unwrap();
        assert_eq!(input, json!({ "op": "list", "limit": 5 }));
    }

    #[test]
    fn key_value_pairs_keep_numbers_booleans_and_plain_text() {
        let input = parse_input(&args(&[
            "op=kill",
            "port=3000",
            "confirm=true",
            "url=https://a.test/?q=1",
        ]))
        .unwrap();
        assert_eq!(
            input,
            json!({ "op": "kill", "port": 3000, "confirm": true, "url": "https://a.test/?q=1" })
        );
    }

    #[test]
    fn no_arguments_is_an_empty_object_and_a_bare_word_is_refused() {
        assert_eq!(parse_input(&[]).unwrap(), json!({}));
        assert!(parse_input(&args(&["list"])).is_err());
    }
}
