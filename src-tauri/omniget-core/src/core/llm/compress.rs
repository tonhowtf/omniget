use serde_json::Value;

const TEXT_MAX: usize = 24 * 1024;
const HEAD_KEEP: usize = 6 * 1024;
const TAIL_KEEP: usize = 16 * 1024;
const MIN_LEN: usize = 600;

fn shape(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_num = false;
    for c in line.trim().chars() {
        if c.is_ascii_digit() {
            if !in_num {
                out.push('#');
                in_num = true;
            }
        } else {
            in_num = false;
            out.push(c);
        }
    }
    out
}

fn cut(s: &str, mut at: usize, forward: bool) -> usize {
    while at < s.len() && !s.is_char_boundary(at) {
        if forward {
            at += 1;
        } else {
            at -= 1;
        }
    }
    at.min(s.len())
}

pub fn text(input: &str) -> String {
    if input.len() < MIN_LEN {
        return input.to_string();
    }
    let lines: Vec<&str> = input.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let key = shape(lines[i]);
        let mut j = i + 1;
        while j < lines.len() && shape(lines[j]) == key {
            j += 1;
        }
        let run = j - i;
        if run >= 4 && !key.is_empty() {
            out.push(lines[i].to_string());
            out.push(format!("[… {} similar lines]", run - 2));
            out.push(lines[j - 1].to_string());
        } else if run >= 3 && key.is_empty() {
            out.push(String::new());
        } else {
            out.extend(lines[i..j].iter().map(|l| l.to_string()));
        }
        i = j;
    }
    let mut joined = out.join("\n");
    if joined.len() > TEXT_MAX {
        let head = cut(&joined, HEAD_KEEP, false);
        let tail = cut(&joined, joined.len() - TAIL_KEEP, true);
        joined = format!(
            "{}\n[… {} bytes omitted]\n{}",
            &joined[..head],
            tail - head,
            &joined[tail..]
        );
    }
    joined
}

fn walk(v: &mut Value) {
    match v {
        Value::String(s) if s.len() >= MIN_LEN => *s = text(s),
        Value::Array(a) => a.iter_mut().for_each(walk),
        Value::Object(o) => o.values_mut().for_each(walk),
        _ => {}
    }
}

pub fn tool_output(tool: &str, content: &str) -> String {
    if content.len() < MIN_LEN
        || matches!(tool, "fs_read" | "fs_edit" | "fs_apply_patch" | "fs_write")
    {
        return content.to_string();
    }
    match serde_json::from_str::<Value>(content) {
        Ok(mut v) if v.is_object() || v.is_array() => {
            walk(&mut v);
            v.to_string()
        }
        _ => text(content),
    }
}
