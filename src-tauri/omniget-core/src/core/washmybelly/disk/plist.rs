use serde_json::{Map, Value};

pub fn parse(xml: &str) -> Result<Value, String> {
    let doc = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .map_err(|e| format!("plist: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "plist" {
        return Err("plist: no <plist> root".into());
    }
    let first = root
        .children()
        .find(|n| n.is_element())
        .ok_or("plist: empty")?;
    Ok(value(first))
}

fn value(n: roxmltree::Node) -> Value {
    let text = || n.text().unwrap_or("").trim().to_string();
    match n.tag_name().name() {
        "dict" => {
            let mut map = Map::new();
            let mut key: Option<String> = None;
            for c in n.children().filter(|c| c.is_element()) {
                if c.tag_name().name() == "key" {
                    key = Some(c.text().unwrap_or("").to_string());
                } else if let Some(k) = key.take() {
                    map.insert(k, value(c));
                }
            }
            Value::Object(map)
        }
        "array" => Value::Array(n.children().filter(|c| c.is_element()).map(value).collect()),
        "string" | "data" | "date" => Value::String(n.text().unwrap_or("").to_string()),
        "integer" => {
            let t = text();
            t.parse::<i64>()
                .map(Value::from)
                .or_else(|_| t.parse::<u64>().map(Value::from))
                .unwrap_or(Value::Null)
        }
        "real" => text()
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = include_str!("../../../../tests/washmybelly_fixtures/disk/root.plist");

    #[test]
    fn reads_diskutil_info() {
        let v = parse(ROOT).unwrap();
        assert_eq!(v["VolumeName"], "Macintosh HD");
        assert_eq!(v["APFSContainerSize"], 994_610_155_520u64);
        assert_eq!(v["APFSSnapshot"], true);
        assert_eq!(v["SolidState"], true);
        assert_eq!(v["APFSPhysicalStores"][0]["APFSPhysicalStore"], "disk0s2");
        assert_eq!(
            v["SMARTDeviceSpecificKeysMayVaryNotGuaranteed"]["TEMPERATURE"],
            325
        );
    }

    #[test]
    fn small_types() {
        let v = parse(r#"<?xml version="1.0"?><plist version="1.0"><dict><key>a</key><real>1.5</real><key>b</key><array><false/><string>x</string></array><key>c</key><dict/></dict></plist>"#).unwrap();
        assert_eq!(v["a"], 1.5);
        assert_eq!(v["b"][0], false);
        assert_eq!(v["b"][1], "x");
        assert!(v["c"].as_object().unwrap().is_empty());
        assert!(parse("<nope/>").is_err());
    }
}
