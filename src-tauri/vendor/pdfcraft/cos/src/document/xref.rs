use super::{Arc, BTreeMap, Lexer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XrefEntry {
    Free { next_generation: u16 },
    InFile { offset: u64, generation: u16 },
    InStream { stream: u32, index: u32 },
}

#[derive(Clone, Copy, Debug)]
pub(super) struct XrefRange {
    pub(super) start: u32,
    pub(super) count: u32,
    pub(super) offset: usize,
    pub(super) stride: usize,
}

#[derive(Clone, Debug)]
pub(super) enum XrefSectionData {
    Table,
    Stream { bytes: Arc<Vec<u8>>, widths: [usize; 3] },
}

#[derive(Clone, Debug)]
pub(super) struct XrefSection {
    pub(super) data: XrefSectionData,
    pub(super) ranges: Vec<XrefRange>,
    pub(super) row_count: usize,
    pub(super) max_num: u32,
    pub(super) has_entries: bool,
    /// A table subsection was read one number lower than it says ([`subsection_start`]).
    pub(super) renumbered: bool,
}

#[derive(Clone, Debug)]
pub(super) struct XrefRevisionSection {
    pub(super) primary: XrefSection,
    pub(super) hybrid: Option<XrefSection>,
}

#[derive(Clone, Debug)]
pub(super) enum XrefIndex {
    Compact { revisions: Vec<XrefRevisionSection>, max_num: u32, has_entries: bool },
    Materialized(Arc<BTreeMap<u32, XrefEntry>>, Arc<Vec<String>>),
}

pub(super) const MAX_COMPACT_XREF_ROWS: usize = 2_000_000;

pub(super) const MAX_COMPACT_XREF_BYTES: usize = 64 * 1024 * 1024;

impl XrefSection {
    pub(super) fn has_any_entry(&self) -> bool {
        match &self.data {
            XrefSectionData::Table => self.has_entries,
            XrefSectionData::Stream { bytes, widths } => {
                let row_width = widths.iter().sum::<usize>();
                if row_width == 0 {
                    return false;
                }
                for range in &self.ranges {
                    let available = bytes.len().saturating_sub(range.offset) / row_width;
                    for i in 0..available.min(range.count as usize) {
                        let Some(offset) = i.checked_mul(row_width).and_then(|delta| range.offset.checked_add(delta)) else { continue };
                        let Some(row) = bytes.get(offset..offset.saturating_add(row_width)) else { continue };
                        let read_field = |from: usize, width: usize, default: u64| -> Option<u64> {
                            if width == 0 {
                                return Some(default);
                            }
                            let end = from.checked_add(width)?;
                            Some(row.get(from..end)?.iter().fold(0u64, |acc, byte| acc.wrapping_shl(8) | u64::from(*byte)))
                        };
                        let Some(kind) = read_field(0, widths[0], 1) else { continue };
                        if kind == 0 || kind == 2 {
                            return true;
                        }
                        if kind == 1 && read_field(widths[0], widths[1], 0).is_some_and(|offset| offset != 0) {
                            return true;
                        }
                    }
                }
                false
            }
        }
    }

    pub(super) fn retained_bytes(&self) -> usize {
        match &self.data {
            XrefSectionData::Table => 0,
            XrefSectionData::Stream { bytes, .. } => bytes.capacity(),
        }
    }

    pub(super) fn lookup(&self, num: u32, file: &[u8]) -> Option<XrefEntry> {
        let after = self.ranges.partition_point(|range| range.start <= num);
        let range = self.ranges.get(after.checked_sub(1)?)?;
        let end = u64::from(range.start).checked_add(u64::from(range.count))?;
        if u64::from(num) >= end {
            return None;
        }
        let row = usize::try_from(num.checked_sub(range.start)?).ok()?;
        let offset = range.offset.checked_add(row.checked_mul(range.stride)?)?;
        match &self.data {
            XrefSectionData::Table => {
                let mut lexer = Lexer::new(file, offset);
                lexer.skip_ws();
                let object_offset = lexer.token();
                lexer.skip_ws();
                let generation = lexer.token();
                lexer.skip_ws();
                let kind = lexer.token();
                match kind {
                    b"n" => {
                        let object_offset = std::str::from_utf8(object_offset).ok()?.parse::<u64>().ok()?;
                        if object_offset == 0 {
                            return None;
                        }
                        let generation = std::str::from_utf8(generation).ok()?.parse::<u32>().unwrap_or(0).min(u16::MAX as u32) as u16;
                        Some(XrefEntry::InFile { offset: object_offset, generation })
                    }
                    b"f" => {
                        let generation = std::str::from_utf8(generation).ok()?.parse::<u32>().unwrap_or(0).min(u16::MAX as u32) as u16;
                        Some(XrefEntry::Free { next_generation: generation })
                    }
                    _ => None,
                }
            }
            XrefSectionData::Stream { bytes, widths } => {
                let row_width = widths.iter().sum::<usize>();
                let end = offset.checked_add(row_width)?;
                let row = bytes.get(offset..end)?;
                let field = |from: usize, width: usize, default: u64| -> Option<u64> {
                    if width == 0 {
                        return Some(default);
                    }
                    let end = from.checked_add(width)?;
                    Some(row.get(from..end)?.iter().fold(0u64, |acc, byte| acc.wrapping_shl(8) | u64::from(*byte)))
                };
                let kind = field(0, widths[0], 1)?;
                let a = field(widths[0], widths[1], 0)?;
                let b = field(widths[0] + widths[1], widths[2], 0)?;
                let entry = match kind {
                    0 => XrefEntry::Free { next_generation: b.min(u16::MAX as u64) as u16 },
                    1 => XrefEntry::InFile { offset: a, generation: b.min(u16::MAX as u64) as u16 },
                    2 => XrefEntry::InStream { stream: a.min(u32::MAX as u64) as u32, index: b.min(u32::MAX as u64) as u32 },
                    _ => return None,
                };
                (!matches!(entry, XrefEntry::InFile { offset: 0, .. })).then_some(entry)
            }
        }
    }

    pub(super) fn numbers(&self) -> impl Iterator<Item = u32> + '_ {
        self.ranges.iter().flat_map(|range| {
            let available = match &self.data {
                XrefSectionData::Table => range.count as usize,
                XrefSectionData::Stream { bytes, widths } => {
                    let row_width = widths.iter().sum::<usize>();
                    bytes.len().saturating_sub(range.offset).checked_div(row_width).unwrap_or(0)
                }
            };
            (0..available.min(range.count as usize)).filter_map(move |i| u32::try_from(i).ok().and_then(|i| range.start.checked_add(i)))
        })
    }

    pub(super) fn extend_numbers(&self, numbers: &mut Vec<u32>) {
        numbers.extend(self.numbers());
    }
}

impl XrefRevisionSection {
    pub(super) fn lookup(&self, num: u32, file: &[u8]) -> Option<XrefEntry> {
        let primary = self.primary.lookup(num, file);
        let hybrid = self.hybrid.as_ref().and_then(|section| section.lookup(num, file));
        match primary {
            None | Some(XrefEntry::Free { .. }) => hybrid.or(primary),
            entry => entry,
        }
    }

    fn extend_numbers(&self, numbers: &mut Vec<u32>) {
        self.primary.extend_numbers(numbers);
        if let Some(hybrid) = &self.hybrid {
            hybrid.extend_numbers(numbers);
        }
    }
}

impl XrefIndex {
    pub(super) fn empty() -> Self {
        Self::Materialized(Arc::new(BTreeMap::new()), Arc::default())
    }

    pub(super) fn lookup(&self, num: u32, file: &[u8]) -> Option<XrefEntry> {
        match self {
            Self::Compact { revisions, .. } => revisions.iter().find_map(|revision| revision.lookup(num, file)),
            Self::Materialized(entries, _) => entries.get(&num).copied(),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        match self {
            Self::Compact { has_entries, .. } => !has_entries,
            Self::Materialized(entries, _) => entries.is_empty(),
        }
    }

    /// Drop an entry for object number 4294967295, which no incremental `/Size` can cover.
    /// Only a materialized index can hold one: the compact reader requires `max_num < /Size`.
    pub(super) fn remove_exhausted(&mut self) -> bool {
        match self {
            Self::Compact { .. } => false,
            Self::Materialized(entries, _) => Arc::make_mut(entries).remove(&u32::MAX).is_some(),
        }
    }

    /// Every entry, newest section first, as the materialized reader keeps them.
    pub(super) fn to_map(&self, file: &[u8]) -> BTreeMap<u32, XrefEntry> {
        match self {
            Self::Materialized(entries, _) => entries.as_ref().clone(),
            Self::Compact { revisions, .. } => {
                let mut numbers = Vec::new();
                for revision in revisions {
                    revision.extend_numbers(&mut numbers);
                }
                numbers.into_iter().filter_map(|num| self.lookup(num, file).map(|entry| (num, entry))).collect()
            }
        }
    }

    pub(super) fn max_num(&self) -> u32 {
        match self {
            Self::Compact { max_num, .. } => *max_num,
            Self::Materialized(entries, _) => entries.keys().next_back().copied().unwrap_or(0),
        }
    }

    pub(super) fn object_numbers(&self, file: &[u8]) -> Vec<u32> {
        match self {
            Self::Materialized(entries, _) => {
                entries.iter().filter_map(|(&num, entry)| (!matches!(entry, XrefEntry::Free { .. })).then_some(num)).collect()
            }
            Self::Compact { revisions, .. } => {
                if let [revision] = revisions.as_slice()
                    && revision.hybrid.is_none()
                {
                    let mut numbers = Vec::with_capacity(revision.primary.row_count);
                    revision.primary.extend_numbers(&mut numbers);
                    numbers.retain(|num| self.lookup(*num, file).is_some_and(|entry| !matches!(entry, XrefEntry::Free { .. })));
                    return numbers;
                }
                let mut numbers = Vec::new();
                for revision in revisions {
                    revision.extend_numbers(&mut numbers);
                }
                numbers.sort_unstable();
                numbers.dedup();
                numbers.retain(|num| self.lookup(*num, file).is_some_and(|entry| !matches!(entry, XrefEntry::Free { .. })));
                numbers
            }
        }
    }
}

/// One cross-reference section of the file (one per save; the first is the original).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revision {
    /// Offset of the xref section (`startxref` value).
    pub xref_offset: u64,
    /// `true` if the section is a cross-reference stream.
    pub is_stream: bool,
    /// `true` if `xref_offset` counts from the `%PDF-` header rather than from the start of the
    /// file. The two differ only in a file with bytes before its header, and an update appended
    /// to such a file must count the same way as the section it points back to.
    pub header_relative: bool,
}
