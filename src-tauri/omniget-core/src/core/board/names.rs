use super::BoardError;

pub const EXTENSION: &str = "excalidraw";
pub const MAX_NAME_CHARS: usize = 120;

const FORBIDDEN: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

pub fn validate(raw: &str) -> Result<String, BoardError> {
    let name = raw.trim();
    let name = strip_extension(name).unwrap_or(name).trim_end();
    if name.is_empty() || name == "." || name == ".." {
        return Err(BoardError::NameEmpty);
    }
    if name
        .chars()
        .any(|c| FORBIDDEN.contains(&c) || c.is_control())
    {
        return Err(BoardError::NameInvalid);
    }
    if name.starts_with('.') || name.ends_with('.') {
        return Err(BoardError::NameInvalid);
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        return Err(BoardError::NameInvalid);
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(BoardError::NameTooLong);
    }
    Ok(name.to_string())
}

pub fn file_name(name: &str) -> String {
    format!("{name}.{EXTENSION}")
}

fn strip_extension(file_name: &str) -> Option<&str> {
    let cut = file_name.len().checked_sub(EXTENSION.len() + 1)?;
    if !file_name.is_char_boundary(cut) {
        return None;
    }
    let (stem, ext) = file_name.split_at(cut);
    (ext.starts_with('.') && ext[1..].eq_ignore_ascii_case(EXTENSION)).then_some(stem)
}

pub fn stem_of(file_name: &str) -> Option<&str> {
    strip_extension(file_name).filter(|s| !s.is_empty())
}

pub fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..10_000)
        .map(|n| format!("{base} {n}"))
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| format!("{base} {}", uuid::Uuid::new_v4().simple()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_and_unicode_names() {
        assert_eq!(validate("  Plano de voo ").unwrap(), "Plano de voo");
        assert_eq!(validate("草图 v2").unwrap(), "草图 v2");
        assert_eq!(validate("idea.excalidraw").unwrap(), "idea");
        assert_eq!(validate("Plan.Excalidraw").unwrap(), "Plan");
    }

    #[test]
    fn stem_ignores_extension_case() {
        assert_eq!(stem_of("Plan.Excalidraw"), Some("Plan"));
        assert_eq!(stem_of("Plan.EXCALIDRAW"), Some("Plan"));
        assert_eq!(stem_of("草图.excalidraw"), Some("草图"));
        assert_eq!(stem_of(".excalidraw"), None);
        assert_eq!(stem_of("plan.txt"), None);
        assert_eq!(stem_of("草"), None);
    }

    #[test]
    fn rejects_separators_and_traversal() {
        for bad in [
            "a/b",
            "a\\b",
            "..",
            ".",
            "../x",
            ".hidden",
            "x:y",
            "tab\there",
            "CON",
            "nul.txt",
        ] {
            assert!(validate(bad).is_err(), "{bad} should be rejected");
        }
        assert!(matches!(validate("   "), Err(BoardError::NameEmpty)));
        assert!(matches!(
            validate(&"a".repeat(121)),
            Err(BoardError::NameTooLong)
        ));
    }

    #[test]
    fn unique_appends_counter() {
        let taken = ["Untitled", "Untitled 2"];
        assert_eq!(unique("Untitled", |n| taken.contains(&n)), "Untitled 3");
        assert_eq!(unique("Fresh", |n| taken.contains(&n)), "Fresh");
    }
}
