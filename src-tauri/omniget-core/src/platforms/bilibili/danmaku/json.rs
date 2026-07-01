use serde_json::json;

use super::proto::DanmakuElem;

pub fn render(elems: &[DanmakuElem]) -> String {
    let arr: Vec<serde_json::Value> = elems
        .iter()
        .map(|e| {
            json!({
                "id": e.id,
                "progress_ms": e.progress_ms,
                "mode": e.mode,
                "fontsize": e.fontsize,
                "color": e.color,
                "mid_hash": e.mid_hash,
                "content": e.content,
                "ctime": e.ctime,
                "weight": e.weight,
                "pool": e.pool,
                "attr": e.attr,
            })
        })
        .collect();
    serde_json::to_string_pretty(&arr).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_json_array() {
        let e = DanmakuElem {
            id: 42,
            progress_ms: 1000,
            content: "x".to_string(),
            ..DanmakuElem::default()
        };
        let s = render(&[e]);
        assert!(s.contains("\"progress_ms\": 1000"));
        assert!(s.contains("\"content\": \"x\""));
    }
}
