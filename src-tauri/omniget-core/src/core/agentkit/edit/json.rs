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

pub fn to_text(value: &Value) -> String {
    format!("{}\n", super::jsonc::pretty(value, "", "  "))
}
