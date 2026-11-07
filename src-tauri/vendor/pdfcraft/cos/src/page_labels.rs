//! Bounded numbering for page labels (ISO 32000-2 §12.4.2).
//! Pure helpers shared by inspection and editing; no object graph or renderer dependency.

/// A complete custom label, including its prefix, may contain this many UTF-8 bytes.
pub const MAX_LABEL_BYTES: usize = 1024;
/// Raw prefixes are bounded before decoding; this admits a full UTF-16BE ASCII label.
pub const MAX_PREFIX_BYTES: usize = 2 * MAX_LABEL_BYTES + 2;
/// Sum of the UTF-8 bytes of custom labels for one document.
pub const MAX_LABEL_TOTAL_BYTES: usize = 4 << 20;
/// Sum of the UTF-8 bytes of the decoded prefixes of one `/PageLabels` tree. Each range decodes
/// its own prefix, and ranges may share one prefix object, so without this a small file could ask
/// for a copy per range: [`MAX_LABEL_TREE_WORK`] ranges naming one 1,024-byte prefix would hold
/// about 200 MiB. Real documents stay far below it.
pub const MAX_PREFIX_TOTAL_BYTES: usize = 4 << 20;
/// The most number-tree entries (nodes, kids and range pairs) read from one `/PageLabels`
/// tree. Scaled for real documents: one range per page on a ten-thousand-page PDF happens
/// (prefix labels; #307's file has 9,156 pages), so the cap is twenty times that while still
/// bounding hostile trees.
pub const MAX_LABEL_TREE_WORK: usize = 200_000;
/// Maximum nesting of a label number tree.
pub const MAX_LABEL_TREE_DEPTH: usize = 32;

/// Lowercase Roman numerals, or `None` if the output exceeds the remaining label budget.
/// The budget is capped at [`MAX_LABEL_BYTES`], including when `n` is hostile.
pub fn roman(mut n: u64, max_bytes: usize) -> Option<String> {
    const PARTS: [(u64, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let max_bytes = max_bytes.min(MAX_LABEL_BYTES);
    let mut out = String::new();
    for (value, part) in PARTS {
        let count = usize::try_from(n / value).ok()?;
        let bytes = count.checked_mul(part.len())?;
        if bytes > max_bytes.saturating_sub(out.len()) {
            return None;
        }
        // The quotient is checked against the tiny byte budget before repeating.
        out.push_str(&part.repeat(count));
        n %= value;
    }
    Some(out)
}

/// a..z, aa..zz, aaa.. (the repeated-letter numbering specified by PDF).
/// Returns `None` instead of truncating a label that exceeds the remaining byte budget.
pub fn alpha(n: u64, max_bytes: usize) -> Option<String> {
    let n = n.max(1) - 1;
    let count = usize::try_from(n / 26 + 1).ok()?;
    if count > max_bytes.min(MAX_LABEL_BYTES) {
        return None;
    }
    let letter = char::from(b'a' + (n % 26) as u8);
    Some(std::iter::repeat_n(letter, count).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_page_labels_follow_pdf_numbering() {
        assert_eq!(roman(4, MAX_LABEL_BYTES).as_deref(), Some("iv"));
        assert_eq!(roman(1994, MAX_LABEL_BYTES).as_deref(), Some("mcmxciv"));
        for (n, expected) in [(0, "a"), (1, "a"), (26, "z"), (27, "aa"), (52, "zz"), (53, "aaa")] {
            assert_eq!(alpha(n, MAX_LABEL_BYTES).as_deref(), Some(expected));
        }
    }

    #[test]
    fn bounded_page_labels_check_lengths_before_repeating() {
        assert_eq!(roman(1_024_000, MAX_LABEL_BYTES).unwrap().len(), MAX_LABEL_BYTES);
        assert!(roman(1_024_001, MAX_LABEL_BYTES).is_none());
        assert_eq!(alpha(26 * 1024, MAX_LABEL_BYTES).unwrap().len(), MAX_LABEL_BYTES);
        assert!(alpha(26 * 1024 + 1, MAX_LABEL_BYTES).is_none());
        assert_eq!(roman(4, 2).as_deref(), Some("iv"));
        assert!(roman(4, 1).is_none());
        assert_eq!(alpha(27, 2).as_deref(), Some("aa"));
        assert!(alpha(27, 1).is_none());
        for max in [0, MAX_LABEL_BYTES, usize::MAX] {
            assert!(roman(u64::MAX, max).is_none());
            assert!(alpha(u64::MAX, max).is_none());
        }
    }
}
