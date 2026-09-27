//! Plain JSON (`.mcp.json`, `settings.json`, `hooks.json`). Same engine as
//! [`super::jsonc`]; a file with comments is still read (some tools tolerate
//! them) and they are kept untouched.

use serde_json::Value;

use super::jsonc::JsoncDoc;
use super::{Editor, Result, Seg, SetOutcome};

#[derive(Debug, Clone)]
pub struct JsonDoc(JsoncDoc);

impl JsonDoc {
    pub fn parse(text: &str) -> Result<JsonDoc> {
        Ok(JsonDoc(JsoncDoc::parse(text)?))
    }
}

impl Editor for JsonDoc {
    fn text(&self) -> &str {
        self.0.text()
    }
    fn get(&self, path: &[Seg]) -> Option<Value> {
        self.0.get(path)
    }
    fn set(&mut self, path: &[Seg], value: &Value) -> Result<SetOutcome> {
        self.0.set(path, value)
    }
    fn remove(&mut self, path: &[Seg]) -> Result<Option<Value>> {
        self.0.remove(path)
    }
    fn push(&mut self, path: &[Seg], value: &Value) -> Result<Option<SetOutcome>> {
        self.0.push(path, value)
    }
    fn remove_item(&mut self, path: &[Seg], value: &Value) -> Result<bool> {
        self.0.remove_item(path, value)
    }
}

/// A whole new JSON file: pretty, two-space indent, trailing newline.
pub fn to_text(value: &Value) -> String {
    format!("{}\n", super::jsonc::pretty(value, "", "  "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agentkit::edit::keys;
    use serde_json::json;

    #[test]
    fn mcp_json_round_trip() {
        let original = "{\n  \"mcpServers\": {\n    \"a\": {\n      \"command\": \"x\"\n    }\n  },\n  \"other\": true\n}\n";
        let mut doc = JsonDoc::parse(original).unwrap();
        let p = keys(["mcpServers", "b"]);
        doc.set(&p, &json!({"type": "http", "url": "https://h"}))
            .unwrap();
        assert!(doc
            .text()
            .contains("\"b\": {\n      \"type\": \"http\",\n      \"url\": \"https://h\"\n    }"));
        doc.remove(&p).unwrap();
        assert_eq!(doc.text(), original);
    }
}
