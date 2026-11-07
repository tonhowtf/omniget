#[cfg(test)]
use super::state::SECTION_READS;
use super::xref::{
    MAX_COMPACT_XREF_BYTES, MAX_COMPACT_XREF_ROWS, Revision, XrefEntry, XrefIndex, XrefRange, XrefRevisionSection, XrefSection, XrefSectionData,
};
use super::{Arc, BTreeMap, CosError, Dict, Document, HashMap, HashSet, Lexer, ObjRef, Object, is_whitespace, parse_indirect, parse_indirect_shared};

impl Document {
    #[allow(clippy::type_complexity)]
    pub(super) fn read_xref_chain(&self, log: &mut Vec<String>) -> Result<(XrefIndex, Dict, Vec<Revision>, Vec<(u64, u32)>), CosError> {
        let mut compact_log = Vec::new();
        if let Ok(Some((index, trailer, revisions))) = self.read_compact_xref_chain(&mut compact_log) {
            let size = trailer.int(b"Size").unwrap_or(0).max(0);
            if size > 0 && size <= i64::from(u32::MAX) && u64::from(index.max_num()) < size as u64 {
                log.extend(compact_log);
                // The compact reader declines any section with set-aside entries, so there are none here.
                return Ok((index, trailer, revisions, Vec::new()));
            }
        }
        let (entries, trailer, revisions, oddities, generation_repairs) = self.read_xref_chain_materialized(log)?;
        Ok((XrefIndex::Materialized(Arc::new(entries), Arc::new(generation_repairs)), trailer, revisions, oddities))
    }

    fn read_compact_xref_chain(&self, log: &mut Vec<String>) -> Result<Option<(XrefIndex, Dict, Vec<Revision>)>, CosError> {
        let data = &self.data;
        let tail_start = data.len().saturating_sub(4096);
        let sx = rfind(data, b"startxref", tail_start).ok_or(CosError::Syntax { offset: data.len(), detail: "no startxref".into() })?;
        let mut lx = Lexer::new(data, sx + 9);
        let first = match lx.object() {
            Ok(Object::Int(n)) if n >= 0 => n as u64,
            _ => return Err(CosError::Syntax { offset: sx, detail: "bad startxref value".into() }),
        };
        let mut sections = Vec::new();
        let mut retained_rows = 0usize;
        let mut retained_bytes = 0usize;
        let mut trailer: Option<Dict> = None;
        let mut revisions = Vec::new();
        let mut visited = HashSet::new();
        let mut next = Some(first);
        while let Some(off) = next.take() {
            if !visited.insert(off) || visited.len() > 10_000 {
                log.push(format!("cross-reference chain loops at offset {off}; stopped"));
                break;
            }
            let ((primary, sect_trailer, is_stream), header_relative) = match self.read_compact_section(off as usize) {
                Ok(Some(v)) => (v, false),
                Ok(None) => return Ok(None),
                Err(e) if self.header_offset > 0 => {
                    log.push(format!("xref at {off} unreadable ({e}); retrying relative to the header"));
                    match self.read_compact_section(off as usize + self.header_offset)? {
                        Some(v) => (v, true),
                        None => return Ok(None),
                    }
                }
                // An unreadable older section loses every object only it locates: the
                // materialized reader logs it and the file is rebuilt from object headers (#1656).
                Err(e) => return Err(e),
            };
            revisions.push(Revision { xref_offset: off, is_stream, header_relative });
            if primary.renumbered {
                log.push(format!("xref subsection at {off} is numbered from 1 but begins with object 0's entry; read from 0"));
            }
            let Some(rows) = retained_rows.checked_add(primary.row_count).filter(|rows| *rows <= MAX_COMPACT_XREF_ROWS) else { return Ok(None) };
            let Some(bytes) = retained_bytes.checked_add(primary.retained_bytes()).filter(|bytes| *bytes <= MAX_COMPACT_XREF_BYTES) else {
                return Ok(None);
            };
            retained_rows = rows;
            retained_bytes = bytes;
            let hybrid = if let Some(x) = sect_trailer.int(b"XRefStm").filter(|x| *x >= 0) {
                match self.read_compact_section(x as usize) {
                    Ok(Some((section, _, _))) => {
                        let Some(rows) = retained_rows.checked_add(section.row_count).filter(|rows| *rows <= MAX_COMPACT_XREF_ROWS) else {
                            return Ok(None);
                        };
                        let Some(bytes) = retained_bytes.checked_add(section.retained_bytes()).filter(|bytes| *bytes <= MAX_COMPACT_XREF_BYTES)
                        else {
                            return Ok(None);
                        };
                        retained_rows = rows;
                        retained_bytes = bytes;
                        Some(section)
                    }
                    Ok(None) => return Ok(None),
                    Err(e) => {
                        log.push(format!("hybrid /XRefStm at {x} unreadable: {e}"));
                        None
                    }
                }
            } else {
                None
            };
            if let Some(hybrid) = &hybrid {
                // A hybrid entry that disagrees with the table is set aside by the materialized reader.
                let mut numbers = Vec::new();
                hybrid.extend_numbers(&mut numbers);
                for num in numbers {
                    let Some(entry) = hybrid.lookup(num, &self.data) else { continue };
                    if primary.lookup(num, &self.data).is_some_and(|old| !matches!(old, XrefEntry::Free { .. }) && old != entry) {
                        return Ok(None);
                    }
                }
            }
            sections.push(XrefRevisionSection { primary, hybrid });
            next = prev_offset(data, &sect_trailer, off, |num| sections.iter().find_map(|s| s.lookup(num, data)), log);
            if trailer.is_none() {
                trailer = Some(sect_trailer);
            }
        }
        revisions.reverse();
        let mut trailer = trailer.ok_or(CosError::Syntax { offset: 0, detail: "no trailer".into() })?;
        for k in [&b"Prev"[..], b"XRefStm", b"Type", b"W", b"Index", b"Filter", b"DecodeParms", b"Length"] {
            trailer.remove(k);
        }
        let mut max_num = 0;
        let mut has_entries = false;
        for revision in &sections {
            max_num = max_num.max(revision.primary.max_num);
            has_entries |= revision.primary.has_entries;
            if let Some(hybrid) = &revision.hybrid {
                max_num = max_num.max(hybrid.max_num);
                has_entries |= hybrid.has_entries;
            }
        }
        Ok(Some((XrefIndex::Compact { revisions: sections, max_num, has_entries }, trailer, revisions)))
    }

    fn read_compact_section(&self, off: usize) -> Result<Option<(XrefSection, Dict, bool)>, CosError> {
        let data = &self.data;
        let mut lx = Lexer::new(data, off);
        if lx.eat_keyword(b"xref") {
            let mut ranges = Vec::new();
            let mut layout_ok = true;
            let mut max_num = 0;
            let mut has_entries = false;
            let mut previous_end = None;
            let mut row_count = 0usize;
            let mut renumbered = false;
            loop {
                lx.skip_ws();
                if lx.eat_keyword(b"trailer") {
                    let t = lx.object()?;
                    let d = t.as_dict().cloned().ok_or(CosError::Syntax { offset: lx.pos, detail: "trailer is not a dictionary".into() })?;
                    if !layout_ok {
                        return Ok(None);
                    }
                    return Ok(Some((XrefSection { data: XrefSectionData::Table, ranges, row_count, max_num, has_entries, renumbered }, d, false)));
                }
                let (Ok(Object::Int(start)), Ok(Object::Int(count))) = (lx.object(), lx.object()) else {
                    return Err(CosError::Syntax { offset: lx.pos, detail: "bad xref subsection header".into() });
                };
                if !valid_xref_range(start, count) || count > 10_000_000 {
                    return Err(CosError::Syntax { offset: lx.pos, detail: "invalid xref object range".into() });
                }
                let shifted = subsection_start(data, self.header_offset, lx.pos, start, count);
                renumbered |= shifted != start;
                let start = shifted;
                let Some(next_rows) = usize::try_from(count).ok().and_then(|count| row_count.checked_add(count)) else { return Ok(None) };
                if next_rows > MAX_COMPACT_XREF_ROWS {
                    return Ok(None);
                }
                row_count = next_rows;
                let mut range_offset = None;
                let mut stride = None;
                let mut previous_record = None;
                for i in 0..count {
                    lx.skip_ws();
                    let record = lx.pos;
                    if range_offset.is_none() {
                        range_offset = Some(record);
                    }
                    if let Some(previous) = previous_record {
                        let Some(delta) = record.checked_sub(previous) else {
                            layout_ok = false;
                            continue;
                        };
                        if stride.is_some_and(|known| known != delta) {
                            layout_ok = false;
                        }
                        stride.get_or_insert(delta);
                    }
                    previous_record = Some(record);
                    let o = lx.token();
                    lx.skip_ws();
                    let _generation = lx.token();
                    lx.skip_ws();
                    let kind = lx.token();
                    let num = (start + i) as u32;
                    let offset = std::str::from_utf8(o).ok().and_then(|s| s.parse::<u64>().ok());
                    match (kind, offset) {
                        (b"n", Some(offset)) if offset != 0 => {
                            has_entries = true;
                            max_num = max_num.max(num);
                        }
                        (b"n", Some(_)) => {
                            // Set aside by the materialized reader, which records it.
                            if num != 0 {
                                return Ok(None);
                            }
                        }
                        (b"f", _) => {
                            has_entries = true;
                            max_num = max_num.max(num);
                        }
                        _ => return Err(CosError::Syntax { offset: lx.pos, detail: "bad xref entry".into() }),
                    }
                }
                if count > 0 {
                    let range_start = start as u32;
                    let range_count = count as u32;
                    let end = u64::from(range_start) + u64::from(range_count);
                    if previous_end.is_some_and(|previous| u64::from(range_start) < previous) {
                        layout_ok = false;
                    }
                    previous_end = Some(end);
                    ranges.push(XrefRange {
                        start: range_start,
                        count: range_count,
                        offset: range_offset.unwrap_or(lx.pos),
                        stride: stride.unwrap_or(1),
                    });
                }
            }
        }
        let (_, obj) = parse_indirect(data, off, &|_| None)?;
        let Object::Stream(stream) = obj else { return Err(CosError::Syntax { offset: off, detail: "expected xref table or stream".into() }) };
        if stream.dict.name(b"Type") != Some(b"XRef") {
            return Err(CosError::Syntax { offset: off, detail: "stream at startxref is not /Type /XRef".into() });
        }
        let w: Vec<usize> = stream
            .dict
            .get(b"W")
            .and_then(Object::as_array)
            .map(|a| a.iter().map(|x| x.as_int().unwrap_or(0).clamp(0, 8) as usize).collect())
            .unwrap_or_default();
        if w.len() < 3 {
            return Err(CosError::Syntax { offset: off, detail: "xref stream /W invalid".into() });
        }
        let widths = [w[0], w[1], w[2]];
        let row = widths.iter().sum::<usize>();
        if row == 0 {
            return Err(CosError::Syntax { offset: off, detail: "xref stream row width 0".into() });
        }
        let size = stream.dict.int(b"Size").unwrap_or(0).max(0);
        let index: Vec<i64> = match stream.dict.get(b"Index").and_then(Object::as_array) {
            Some(a) => a.iter().filter_map(Object::as_int).collect(),
            None => vec![0, size],
        };
        let mut requested_rows = 0usize;
        let mut previous_end = None;
        for pair in index.chunks(2) {
            let [start, count] = pair else { break };
            if !valid_xref_range(*start, *count) {
                return Err(CosError::Syntax { offset: off, detail: "invalid xref object range".into() });
            }
            let Ok(count) = usize::try_from(*count) else { return Ok(None) };
            let Some(next) = requested_rows.checked_add(count).filter(|rows| *rows <= MAX_COMPACT_XREF_ROWS) else { return Ok(None) };
            if count > 0 {
                let end = (*start as u64).saturating_add(count as u64);
                if previous_end.is_some_and(|previous| (*start as u64) < previous) {
                    return Ok(None);
                }
                previous_end = Some(end);
            }
            requested_rows = next;
        }
        if requested_rows.checked_mul(row).is_none_or(|bytes| bytes > MAX_COMPACT_XREF_BYTES) {
            return Ok(None);
        }
        let raw = stream.decoded_within(MAX_XREF_OBJECTS.saturating_mul(row).min(self.stream_limit).min(MAX_COMPACT_XREF_BYTES))?;
        if raw.capacity() > MAX_COMPACT_XREF_BYTES {
            return Ok(None);
        }
        let mut ranges = Vec::new();
        let mut row_start = 0usize;
        let available_rows = raw.len() / row;
        for pair in index.chunks(2) {
            let [start, count] = pair else { break };
            let range_start = *start as u32;
            let Ok(requested) = usize::try_from(*count) else { return Ok(None) };
            let range_count = requested.min(available_rows.saturating_sub(row_start));
            if range_count > 0 {
                let Ok(range_count_u32) = u32::try_from(range_count) else { return Ok(None) };
                let Some(byte_offset) = row_start.checked_mul(row) else { return Ok(None) };
                ranges.push(XrefRange { start: range_start, count: range_count_u32, offset: byte_offset, stride: row });
            }
            let Some(next) = row_start.checked_add(range_count) else { return Ok(None) };
            row_start = next;
        }
        // Rows the materialized reader sets aside (reserved types, offset 0) are left to it.
        for range in &ranges {
            for i in 0..range.count as usize {
                let at = range.offset + i * row;
                let Some(r) = raw.get(at..at + row) else { break };
                let field = |from: usize, width: usize, default: u64| -> u64 {
                    if width == 0 {
                        return default;
                    }
                    r[from..from + width].iter().fold(0u64, |acc, byte| acc << 8 | u64::from(*byte))
                };
                let kind = field(0, widths[0], 1);
                let offset = field(widths[0], widths[1], 0);
                if kind > 2 || (kind == 1 && offset == 0 && range.start + i as u32 != 0) {
                    return Ok(None);
                }
            }
        }
        let max_num = ranges.iter().filter_map(|range| range.start.checked_add(range.count.saturating_sub(1))).max().unwrap_or(0);
        let row_count = row_start;
        let section = XrefSection {
            data: XrefSectionData::Stream { bytes: Arc::new(raw), widths },
            ranges,
            row_count,
            max_num,
            has_entries: false,
            renumbered: false,
        };
        let has_entries = section.has_any_entry();
        let section = XrefSection { has_entries, ..section };
        Ok(Some((section, stream.dict.clone(), true)))
    }

    #[allow(clippy::type_complexity)]
    pub(super) fn read_xref_chain_materialized(
        &self,
        log: &mut Vec<String>,
    ) -> Result<(BTreeMap<u32, XrefEntry>, Dict, Vec<Revision>, Vec<(u64, u32)>, Vec<String>), CosError> {
        let data = &self.data;
        let tail_start = data.len().saturating_sub(4096);
        let sx = rfind(data, b"startxref", tail_start).ok_or(CosError::Syntax { offset: data.len(), detail: "no startxref".into() })?;
        let mut lx = Lexer::new(data, sx + 9);
        let first = match lx.object() {
            Ok(Object::Int(n)) if n >= 0 => n as u64,
            _ => return Err(CosError::Syntax { offset: sx, detail: "bad startxref value".into() }),
        };
        let mut entries = BTreeMap::new();
        let mut trailer: Option<Dict> = None;
        let mut revisions = Vec::new();
        let mut oddities = Vec::new();
        let mut visited = HashSet::new();
        // Hybrid streams already read, by offset: their entries, or `None` when unreadable. A newer
        // section has merged every entry of one into `entries`, so an older section that names it
        // again only has its own table compared with them, without decoding the stream again.
        // At most MAX_XREF_OBJECTS entries are kept in all; a stream past that is read again.
        let mut hybrids: HashMap<i64, Option<BTreeMap<u32, XrefEntry>>> = HashMap::new();
        let (mut hybrid_rows, mut noted) = (0usize, HashSet::new());
        let mut generation_repairs = Vec::new();
        let mut recorded = HashSet::new();
        let mut diagnose = true;
        let mut next = Some(first);
        while let Some(off) = next.take() {
            if !visited.insert(off) || visited.len() > 10_000 {
                log.push(format!("cross-reference chain loops at offset {off}; stopped"));
                break;
            }
            // This section's own entries; older sections only fill what newer ones left out.
            let mut sect = BTreeMap::new();
            // Object numbers whose entry in this section was set aside ([`Self::xref_oddities`]).
            let mut odd = Vec::new();
            let ((sect_trailer, is_stream), header_relative) = match self.read_section(off as usize, &mut sect, &mut odd, log) {
                Ok(v) => (v, false),
                Err(e) if self.header_offset > 0 => {
                    log.push(format!("xref at {off} unreadable ({e}); retrying relative to the header"));
                    sect.clear();
                    odd.clear();
                    (self.read_section(off as usize + self.header_offset, &mut sect, &mut odd, log)?, true)
                }
                Err(e) => {
                    // An unreadable older section loses every object only it locates (the page
                    // tree, often): rebuild from object headers, as readers do (#1656).
                    if !revisions.is_empty() {
                        log.push(format!("older cross-reference section at {off} is unreadable: {e}"));
                    }
                    return Err(e);
                }
            };
            revisions.push(Revision { xref_offset: off, is_stream, header_relative });
            for (&num, &entry) in &sect {
                if !diagnose {
                    break;
                }
                diagnose = self.inspect_xref_generation_repair(num, entry, &mut recorded, &mut generation_repairs);
            }
            // Hybrid files (§7.5.8.4): the /XRefStm entries supplement the table and replace the
            // free entries it gives objects only a cross-reference stream can locate.
            let hybrid = sect_trailer.int(b"XRefStm").filter(|x| *x >= 0);
            if let Some((x, read)) = hybrid.and_then(|x| hybrids.get(&x).map(|read| (x, read))) {
                if noted.insert(x) {
                    log.push(format!("hybrid /XRefStm at {x} named again by the section at {off}; read once"));
                }
                if let Some(stm) = read {
                    for (num, old) in &sect {
                        // The table places the object somewhere other than the stream does.
                        if !matches!(old, XrefEntry::Free { .. }) && stm.get(num).is_some_and(|e| e != old) {
                            odd.push(*num);
                        }
                    }
                }
            } else if let Some(x) = hybrid {
                let mut stm = BTreeMap::new();
                let read = match self.read_section(x as usize, &mut stm, &mut odd, log) {
                    Ok(_) => {
                        for (&num, &entry) in &stm {
                            if !diagnose {
                                break;
                            }
                            diagnose = self.inspect_xref_generation_repair(num, entry, &mut recorded, &mut generation_repairs);
                        }
                        for (&num, &e) in &stm {
                            match sect.get(&num) {
                                None | Some(XrefEntry::Free { .. }) => {
                                    sect.insert(num, e);
                                }
                                // The table already places the object somewhere else.
                                Some(old) if *old != e => odd.push(num),
                                Some(_) => {}
                            }
                        }
                        Some(stm)
                    }
                    Err(e) => {
                        log.push(format!("hybrid /XRefStm at {x} unreadable: {e}"));
                        None
                    }
                };
                let rows = read.as_ref().map_or(0, BTreeMap::len);
                if let Some(total) = hybrid_rows.checked_add(rows).filter(|total| *total <= MAX_XREF_OBJECTS) {
                    hybrid_rows = total;
                    hybrids.insert(x, read);
                }
            }
            oddities.extend(odd.into_iter().map(|num| (off, num)));
            for (num, e) in sect {
                entries.entry(num).or_insert(e);
            }
            next = prev_offset(data, &sect_trailer, off, |num| entries.get(&num).copied(), log);
            if trailer.is_none() {
                trailer = Some(sect_trailer);
            }
        }
        revisions.reverse();
        let mut trailer = trailer.ok_or(CosError::Syntax { offset: 0, detail: "no trailer".into() })?;
        for k in [&b"Prev"[..], b"XRefStm", b"Type", b"W", b"Index", b"Filter", b"DecodeParms", b"Length"] {
            trailer.remove(k);
        }
        Ok((entries, trailer, revisions, oddities, generation_repairs))
    }

    /// Read one section at `off`, adding entries not already known (newer sections win). `odd`
    /// collects the object numbers whose entry here is set aside ([`Self::xref_oddities`]).
    fn read_section(
        &self,
        off: usize,
        entries: &mut BTreeMap<u32, XrefEntry>,
        odd: &mut Vec<u32>,
        log: &mut Vec<String>,
    ) -> Result<(Dict, bool), CosError> {
        #[cfg(test)]
        SECTION_READS.with(|c| c.set(c.get() + 1));
        let data = &self.data;
        let mut lx = Lexer::new(data, off);
        if lx.eat_keyword(b"xref") {
            loop {
                lx.skip_ws();
                if lx.eat_keyword(b"trailer") {
                    let t = lx.object()?;
                    let d = t.as_dict().cloned().ok_or(CosError::Syntax { offset: lx.pos, detail: "trailer is not a dictionary".into() })?;
                    return Ok((d, false));
                }
                let (Ok(Object::Int(start)), Ok(Object::Int(count))) = (lx.object(), lx.object()) else {
                    return Err(CosError::Syntax { offset: lx.pos, detail: "bad xref subsection header".into() });
                };
                if !valid_xref_range(start, count) || count > 10_000_000 {
                    return Err(CosError::Syntax { offset: lx.pos, detail: "invalid xref object range".into() });
                }
                let shifted = subsection_start(data, self.header_offset, lx.pos, start, count);
                if shifted != start {
                    log.push(format!("xref subsection at {off} is numbered from 1 but begins with object 0's entry; read from 0"));
                }
                let start = shifted;
                for i in 0..count {
                    lx.skip_ws();
                    let o = lx.token();
                    lx.skip_ws();
                    let g = lx.token();
                    lx.skip_ws();
                    let kind = lx.token();
                    let num = (start + i) as u32;
                    let offset = std::str::from_utf8(o).ok().and_then(|s| s.parse::<u64>().ok());
                    let generation = std::str::from_utf8(g).ok().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0).min(u16::MAX as u32) as u16;
                    let entry = match (kind, offset) {
                        (b"n", Some(offset)) => XrefEntry::InFile { offset, generation },
                        (b"f", _) => XrefEntry::Free { next_generation: generation },
                        _ => return Err(CosError::Syntax { offset: lx.pos, detail: "bad xref entry".into() }),
                    };
                    // Offset 0 "in use" entries are a known producer bug: ignore them. (Object 0
                    // is never a real object, so its entry cannot mislead anyone.)
                    if matches!(entry, XrefEntry::InFile { offset: 0, .. }) {
                        if num != 0 {
                            odd.push(num);
                        }
                        continue;
                    }
                    if *entries.entry(num).or_insert(entry) != entry {
                        odd.push(num);
                    }
                }
            }
        }
        // Cross-reference stream.
        let (_, obj) = parse_indirect(data, off, &|_| None)?;
        let Object::Stream(s) = obj else { return Err(CosError::Syntax { offset: off, detail: "expected xref table or stream".into() }) };
        if s.dict.name(b"Type") != Some(b"XRef") {
            return Err(CosError::Syntax { offset: off, detail: "stream at startxref is not /Type /XRef".into() });
        }
        let w: Vec<usize> = s
            .dict
            .get(b"W")
            .and_then(Object::as_array)
            .map(|a| a.iter().map(|x| x.as_int().unwrap_or(0).clamp(0, 8) as usize).collect())
            .unwrap_or_default();
        if w.len() < 3 {
            return Err(CosError::Syntax { offset: off, detail: "xref stream /W invalid".into() });
        }
        let size = s.dict.int(b"Size").unwrap_or(0).max(0);
        let index: Vec<i64> = match s.dict.get(b"Index").and_then(Object::as_array) {
            Some(a) => a.iter().filter_map(Object::as_int).collect(),
            None => vec![0, size],
        };
        let row = w[0] + w[1] + w[2];
        if row == 0 {
            return Err(CosError::Syntax { offset: off, detail: "xref stream row width 0".into() });
        }
        // One row per object, and a document has at most 8,388,607 indirect objects (the classic
        // implementation limit): decoding more than that (about 200 MB at the widest rows) is a
        // decompression bomb, not cross-reference data. The section is then reconstructed.
        let raw = s.decoded_within(MAX_XREF_OBJECTS.saturating_mul(row).min(self.stream_limit))?;
        let field = |r: &[u8], from: usize, len: usize, default: u64| -> u64 {
            if len == 0 {
                return default;
            }
            r[from..from + len].iter().fold(0u64, |acc, b| acc << 8 | *b as u64)
        };
        let mut rows = raw.chunks_exact(row);
        for pair in index.chunks(2) {
            let [start, count] = pair else { break };
            if !valid_xref_range(*start, *count) {
                return Err(CosError::Syntax { offset: off, detail: "invalid xref object range".into() });
            }
            for i in 0..*count {
                let Some(r) = rows.next() else { break };
                let t = field(r, 0, w[0], 1);
                let a = field(r, w[0], w[1], 0);
                let b = field(r, w[0] + w[1], w[2], 0);
                let num = (*start + i) as u32;
                let entry = match t {
                    0 => XrefEntry::Free { next_generation: b.min(u16::MAX as u64) as u16 },
                    1 => XrefEntry::InFile { offset: a, generation: b.min(u16::MAX as u64) as u16 },
                    2 => XrefEntry::InStream { stream: a.min(u32::MAX as u64) as u32, index: b.min(u32::MAX as u64) as u32 },
                    // Reserved types are treated as null references (§7.5.8.3).
                    _ => {
                        odd.push(num);
                        continue;
                    }
                };
                if matches!(entry, XrefEntry::InFile { offset: 0, .. }) {
                    if num != 0 {
                        odd.push(num);
                    }
                    continue;
                }
                if *entries.entry(num).or_insert(entry) != entry {
                    odd.push(num);
                }
            }
        }
        Ok((s.dict.clone(), true))
    }

    /// Cross-reference data that leaves out objects the file contains: a subsection numbered one
    /// too low (object 0 in use, the last object without an entry), or an in-use entry at offset 0
    /// that nothing else replaces. (A number no section lists is not evidence: it is a null
    /// object, §7.3.10, and valid files have them below an oversized `/Size`.) Viewers rebuild
    /// the table and show these objects; reading them as null would drop them from every copy.
    /// When one of those shows, the file is scanned once for object headers, and wrong offsets
    /// are looked up there (`load`). Only the numbers the damage leaves out take the last
    /// header's location: one whose entry was set aside, or the last object of a subsection
    /// numbered one too low. Any other unlisted number stays null, and existing
    /// entries are never replaced: an object freed by an update stays freed.
    pub(super) fn fill_xref_gaps(&mut self, log: &mut Vec<String>) {
        let entries = &self.entries;
        let shifted = matches!(entries.lookup(0, &self.data), Some(XrefEntry::InFile { .. }));
        let set_aside = self.xref_oddities.iter().any(|(_, num)| entries.lookup(*num, &self.data).is_none());
        if !(shifted || set_aside) {
            return;
        }
        let mut located = BTreeMap::new();
        for (num, offset, generation, _) in object_headers(&self.data) {
            located.insert(num, (offset, generation));
        }
        let mut filled = entries.to_map(&self.data);
        // A subsection `0 N` whose rows are those of objects 1 to N leaves out object N, whatever
        // newer sections say about its neighbours.
        let past_shift: HashSet<u32> = if shifted {
            self.revisions
                .iter()
                .filter(|r| !r.is_stream)
                .filter_map(|r| usize::try_from(r.xref_offset).ok().map(|o| if r.header_relative { o.saturating_add(self.header_offset) } else { o }))
                .filter_map(|at| shifted_subsection_end(&self.data, at))
                .collect()
        } else {
            HashSet::new()
        };
        let odd: HashSet<u32> = self.xref_oddities.iter().map(|(_, n)| *n).collect();
        let mut found = 0usize;
        for (&num, &(offset, generation)) in &located {
            let left_out = past_shift.contains(&num) || odd.contains(&num);
            if num != 0 && !filled.contains_key(&num) && left_out {
                filled.insert(num, XrefEntry::InFile { offset, generation });
                found += 1;
            }
        }
        if found > 0 {
            log.push(format!("the cross-reference data leaves out {found} objects the file contains; located them by their object headers"));
            self.entries = Arc::new(XrefIndex::Materialized(Arc::new(filled), Arc::default()));
        }
        self.located = Arc::new(located);
    }

    /// Rebuild the cross-reference index by scanning for `N G obj` headers (§7.5.4 recovery).
    pub(super) fn reconstruct(&mut self, log: &mut Vec<String>) -> Result<(), CosError> {
        let data = self.data.clone();
        let mut entries: BTreeMap<u32, XrefEntry> = BTreeMap::new();
        let mut objstms = Vec::new();
        // Offsets of objects that may be cross-reference streams: their dictionaries are the
        // trailers of files that have no `trailer` keyword (§7.5.8).
        let mut xref_streams = Vec::new();
        for (num, offset, generation, body) in object_headers(&data) {
            entries.insert(num, XrefEntry::InFile { offset, generation });
            objstms.push(num);
            if find(&data, b"/XRef", body, body.saturating_add(1024)).is_some()
                && let Ok(at) = usize::try_from(offset)
            {
                xref_streams.push(at);
            }
        }
        if entries.is_empty() {
            return Err(CosError::Syntax { offset: 0, detail: "no objects found".into() });
        }
        self.entries = Arc::new(XrefIndex::Materialized(Arc::new(entries.clone()), Arc::default()));
        self.cache.lock().map_err(|_| CosError::Poisoned)?.clear();
        // Objects inside object streams.
        for num in objstms {
            if let Ok(o) = self.try_get(num)
                && let Object::Stream(s) = &*o
                && s.dict.name(b"Type") == Some(b"ObjStm")
                && let Ok(stm) = self.objstm(num)
            {
                for (idx, member) in stm.index.iter().enumerate() {
                    if let Some((inner, _)) = member {
                        entries.entry(*inner).or_insert(XrefEntry::InStream { stream: num, index: idx as u32 });
                    }
                }
            }
        }
        let object_count = entries.len();
        self.entries = Arc::new(XrefIndex::Materialized(Arc::new(entries), Arc::default()));
        self.cache.lock().map_err(|_| CosError::Poisoned)?.clear();
        // Trailer: the last `trailer` dictionary or cross-reference stream dictionary with a
        // usable /Root (last in the file, whichever kind), else the catalog itself. One whose
        // /Root is a catalog (as `root_is_catalog` judges it) beats one whose /Root is another
        // dictionary, such as /Info, wherever it is; an FDF's /Root (no /Type, no /Pages) still
        // counts when nothing better is there.
        // A file whose only trailers are cross-reference streams (and whose `startxref` is gone
        // or points nowhere) keeps its /Info, /ID and /Encrypt this way.
        let rank = |at: usize, d: &Dict| {
            let root = self.try_get_reference(d.reference(b"Root")?).ok()?;
            let Object::Dict(c) = &*root else { return None };
            Some((c.name(b"Type") == Some(b"Catalog") || c.contains(b"Pages"), at))
        };
        let mut trailer: Option<((bool, usize), Dict)> = None;
        let mut from = 0;
        while let Some(p) = find(&data, b"trailer", from, data.len()) {
            from = p + 7;
            let mut lx = Lexer::new(&data, from);
            if let Ok(Object::Dict(d)) = lx.object()
                && let Some(r) = rank(p, &d)
                && trailer.as_ref().is_none_or(|(best, _)| r > *best)
            {
                trailer = Some((r, d));
            }
        }
        for at in xref_streams {
            if trailer.as_ref().is_some_and(|((catalog, p), _)| *catalog && *p > at) {
                continue;
            }
            if let Ok((_, Object::Stream(s))) = parse_indirect_shared(&data, at, &|_| None, &self.endstreams)
                && s.dict.name(b"Type") == Some(b"XRef")
                && let Some(r) = rank(at, &s.dict)
                && trailer.as_ref().is_none_or(|(best, _)| r > *best)
            {
                trailer = Some((r, s.dict));
            }
        }
        let trailer = match trailer {
            Some((_, t)) => t,
            None => {
                let catalog = self
                    .entries
                    .object_numbers(&self.data)
                    .into_iter()
                    .find(|n| matches!(self.try_get(*n).as_deref(), Ok(Object::Dict(d)) if d.name(b"Type") == Some(b"Catalog")));
                let Some(c) = catalog else { return Err(CosError::Syntax { offset: 0, detail: "no document catalog found".into() }) };
                let mut t = Dict::new();
                t.set(b"Root".to_vec(), Object::Ref(ObjRef::new(c, self.generation(c))));
                t
            }
        };
        let mut trailer = trailer;
        // What a cross-reference stream's dictionary carries for itself goes, as the chain
        // readers drop it: a save writes its own.
        for k in [&b"Prev"[..], b"XRefStm", b"Type", b"W", b"Index", b"Filter", b"DecodeParms", b"Length"] {
            trailer.remove(k);
        }
        self.trailer = trailer;
        self.revisions = Arc::new(Vec::new());
        log.push(format!("rebuilt cross-reference index from {object_count} object headers"));
        Ok(())
    }
}

/// For a classic table at `at` whose first subsection is `0 N` and opens with an in-use row
/// (the rows of objects 1 to N), the number N its rows really end at.
fn shifted_subsection_end(data: &[u8], at: usize) -> Option<u32> {
    let mut lx = Lexer::new(data, at);
    if !lx.eat_keyword(b"xref") {
        return None;
    }
    let (Ok(Object::Int(0)), Ok(Object::Int(count))) = (lx.object(), lx.object()) else { return None };
    lx.skip_ws();
    let offset = std::str::from_utf8(lx.token()).ok().and_then(|s| s.parse::<u64>().ok());
    lx.skip_ws();
    lx.token();
    lx.skip_ws();
    match (offset, lx.token()) {
        (Some(offset), b"n") if offset != 0 => u32::try_from(count).ok().filter(|n| *n > 0),
        _ => None,
    }
}

/// Every `N G obj` header in `data`, in file order, as (number, offset, generation, position
/// after `obj`).
fn object_headers(data: &[u8]) -> Vec<(u32, u64, u16, usize)> {
    let mut headers = Vec::new();
    let mut i = 0;
    while i < data.len() {
        // Object headers start at a line start (or after whitespace).
        let at_start = i == 0 || is_whitespace(data[i - 1]) || data[i - 1] == b'>';
        if at_start && data[i].is_ascii_digit() {
            let mut lx = Lexer::new(data, i);
            let n = lx.token();
            lx.skip_ws();
            let g = lx.token();
            if !n.is_empty() && !g.is_empty() && n.iter().all(u8::is_ascii_digit) && g.iter().all(u8::is_ascii_digit) && lx.eat_keyword(b"obj") {
                let num = std::str::from_utf8(n).ok().and_then(|s| s.parse::<u32>().ok());
                let generation = std::str::from_utf8(g).ok().and_then(|s| s.parse::<u16>().ok()).unwrap_or(0);
                if let Some(num) = num {
                    headers.push((num, i as u64, generation, lx.pos));
                }
                i = lx.pos;
                continue;
            }
        }
        i += 1;
    }
    headers
}

/// The offset of the section before the one whose trailer this is (`/Prev`), if any. Some writers
/// give it as a reference, `N 0 R`. When the sections read so far (`lookup`) place that object
/// and it holds an offset (iText's `indirectPrev.pdf`), that is the offset; otherwise, as the
/// tiff2pdf files in JHOVE's PDF-HUL-157 cases need, the sections before are at N. Either way the
/// reference is noted. Any other value that is not an offset is noted and ends the chain.
fn prev_offset(data: &[u8], trailer: &Dict, off: u64, lookup: impl Fn(u32) -> Option<XrefEntry>, log: &mut Vec<String>) -> Option<u64> {
    match trailer.get(b"Prev") {
        Some(Object::Int(p)) if *p >= 0 => Some(*p as u64),
        Some(Object::Ref(r)) => {
            let held = match lookup(r.num) {
                Some(XrefEntry::InFile { offset, generation }) if generation == r.generation => {
                    usize::try_from(offset).ok().and_then(|at| parse_indirect(data, at, &|_| None).ok()).and_then(|(at, value)| match value {
                        Object::Int(p) if at == *r && p >= 0 => Some(p as u64),
                        _ => None,
                    })
                }
                _ => None,
            };
            match held {
                Some(p) => {
                    log.push(format!(
                        "the trailer at {off} gives /Prev as the reference {} {} R; reading the offset {p} it holds",
                        r.num, r.generation
                    ));
                    Some(p)
                }
                None => {
                    log.push(format!(
                        "the trailer at {off} gives /Prev as the reference {} {} R; reading it as offset {}",
                        r.num, r.generation, r.num
                    ));
                    Some(u64::from(r.num))
                }
            }
        }
        Some(_) => {
            log.push(format!("the trailer at {off} has a /Prev that is not an offset; the sections before it are unreachable"));
            None
        }
        None => None,
    }
}

pub(crate) fn find(hay: &[u8], needle: &[u8], from: usize, to: usize) -> Option<usize> {
    let to = to.min(hay.len());
    if from >= to || needle.is_empty() || to - from < needle.len() {
        return None;
    }
    hay[from..to].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

pub(super) fn rfind(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).rev().find(|&i| &hay[i..i + needle.len()] == needle)
}

/// The object number a classic xref subsection really starts at. A common producer bug heads the
/// table `1 n` and then writes object 0's free entry (`0000000000 65535 f`) as its first row, so
/// every row would be read one number too high and object 1 as free. Such a subsection is read
/// from 0 when its second row, read that way, locates an object that is indeed numbered 1, at
/// that offset or, as `load` reads offsets, relative to a header shifted by `header_offset`.
/// `rows` is the position just after the subsection header.
fn subsection_start(data: &[u8], header_offset: usize, rows: usize, start: i64, count: i64) -> i64 {
    if start != 1 || count < 2 {
        return start;
    }
    let mut lx = Lexer::new(data, rows);
    let mut row = || {
        lx.skip_ws();
        let offset = std::str::from_utf8(lx.token()).ok().and_then(|s| s.parse::<u64>().ok());
        lx.skip_ws();
        let generation = std::str::from_utf8(lx.token()).ok().and_then(|s| s.parse::<u32>().ok());
        lx.skip_ws();
        (offset, generation, lx.token() == b"f")
    };
    let (Some(0), Some(65535), true) = row() else { return start };
    let (Some(second), _, false) = row() else { return start };
    let object_one = |at: usize| {
        if at >= data.len() {
            return false;
        }
        let mut obj = Lexer::new(data, at);
        matches!((obj.object(), obj.object()), (Ok(Object::Int(1)), Ok(Object::Int(_)))) && obj.eat_keyword(b"obj")
    };
    let Ok(second) = usize::try_from(second) else { return start };
    if object_one(second) || (header_offset != 0 && object_one(second.saturating_add(header_offset))) { 0 } else { start }
}

/// Object numbers must fit the representation used by references and the xref map.
/// Validate the whole subsection before adding entries, so neither arithmetic overflow nor
/// truncation can turn a damaged range into entries for unrelated objects.
fn valid_xref_range(start: i64, count: i64) -> bool {
    u32::try_from(start).is_ok() && count >= 0 && start.checked_add(count).is_some_and(|end| end <= i64::from(u32::MAX) + 1)
}

/// The most indirect objects a document can have (the classic implementation limit, ISO
/// 32000-1 Annex C): it bounds how much data a cross-reference stream can hold.
const MAX_XREF_OBJECTS: usize = 8_388_607;
