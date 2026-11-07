//! JBIG2 decoding for `JBIG2Decode` (ISO 32000-2 §7.4.7), through `hayro-jbig2`.

use crate::FilterError;
use crate::ccitt::Rows;

/// The largest JBIG2 page [`decode_jbig2_within`] decodes, in pixels. The decoder decodes a page whole,
/// and takes pages of up to this many (20,000 × 10,000 decodes). It is also the most
/// [`encode_jbig2_generic`](crate::encode_jbig2_generic) encodes, so what it stores reads back.
pub const MAX_JBIG2_PIXELS: u64 = 1 << 28;

/// A `JBIG2Decode` stream's `data`, with what its `/JBIG2Globals` stream decodes to, decoded as the
/// filter gives it: `rows` rows of `columns` 1-bit samples, most significant bit first, each
/// starting on a new byte, with black as 0 (JBIG2 codes black as 1).
///
/// The JBIG2 page has to be the image's size, `columns` × `rows`: PdfCraft's renderer draws the
/// whole page into the image, so the rows of a page of another height wouldn't be the image's. The
/// decoder decodes the page whole, so it is checked before it is decoded, and has to fit in `max`
/// bytes as such rows and have at most [`MAX_JBIG2_PIXELS`]. Regions inside it are bounded by the
/// decoder alone: a region may claim more pixels than its page, and is decoded before it is cut to
/// the page. With no rows asked for, nothing is decoded.
///
/// Errors: `data` or `globals` that don't decode, or a page of another size
/// ([`FilterError::Corrupt`]); a page past `max` ([`FilterError::LimitExceeded`]) or
/// [`MAX_JBIG2_PIXELS`] ([`FilterError::Unsupported`], or `Corrupt` from the decoder, which has the
/// same cap and can refuse the page first).
///
/// Decoding spends from `work`: what decoding may still cost across a run of images, in the
/// decoder's count of steps. There is no way in without it, so a run of images can't decode them
/// without sharing an allowance. Reading `data` and `globals` is charged first
/// ([`hayro_jbig2::charge_reading`]), so an image that turns out not to decode has paid for that,
/// then what decoding takes ([`hayro_jbig2::Image::decode_within`]). An image takes at most what
/// one decode may on top of its reading and its page, whatever `work` allows, and one that `work`
/// can't pay for spends all of it. With nothing left an image isn't read at all, so the images after
/// that one cost nothing but this check, however large their globals.
pub fn decode_jbig2_within(data: &[u8], globals: Option<&[u8]>, columns: u32, rows: u32, max: usize, work: &mut u64) -> Result<Vec<u8>, FilterError> {
    if rows == 0 {
        return Ok(Vec::new());
    }
    if *work == 0 {
        return Err(FilterError::Spent);
    }
    hayro_jbig2::charge_reading(work, data.len().saturating_add(globals.map_or(0, <[u8]>::len))).map_err(|_| FilterError::Spent)?;
    let corrupt = |detail: String| FilterError::Corrupt { filter: "JBIG2Decode", detail };
    let image = hayro_jbig2::Image::new_embedded(data, globals).map_err(|e| corrupt(e.to_string()))?;
    let (width, height) = (image.width(), image.height());
    if (width, height) != (columns, rows) {
        return Err(corrupt(format!("a page of {width} × {height} pixels for an image of {columns} × {rows}")));
    }
    if u64::from(width) * u64::from(height) > MAX_JBIG2_PIXELS {
        return Err(FilterError::Unsupported(format!("JBIG2 pages of {width} × {height} pixels")));
    }
    // u32 to usize: wasm32 and up have at least 32-bit usize.
    let (width, height) = (width as usize, height as usize);
    if width.div_ceil(8).checked_mul(height).is_none_or(|bytes| bytes > max) {
        return Err(FilterError::LimitExceeded(max));
    }
    let mut out = Rows::new(width, height, false);
    // Stopped by what was left, it isn't a damaged image, whatever the decoder calls it.
    image.decode_within(&mut out, work).map_err(|e| if *work == 0 { FilterError::Spent } else { corrupt(e.to_string()) })?;
    Ok(out.out)
}

/// JBIG2 codes black as 1, which `JBIG2Decode` gives as 0.
impl hayro_jbig2::Decoder for Rows {
    fn push_pixel(&mut self, black: bool) {
        self.push(!black, 1);
    }
    fn push_pixel_chunk(&mut self, black: bool, chunks: u32) {
        self.push(!black, (chunks as usize).saturating_mul(8));
    }
    fn next_line(&mut self) {
        self.end_row();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{encode_jbig2, encode_jbig2_generic};

    /// [`decode_jbig2_within`] with no limit on the work.
    fn decode_jbig2(data: &[u8], globals: Option<&[u8]>, columns: u32, rows: u32, max: usize) -> Result<Vec<u8>, FilterError> {
        let mut unbounded = u64::MAX;
        decode_jbig2_within(data, globals, columns, rows, max, &mut unbounded)
    }

    /// `rows` rows of `columns` samples of noise, the bits that pad each row to a byte 0.
    fn noise(columns: u32, rows: u32) -> Vec<u8> {
        let stride = (columns as usize).div_ceil(8);
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut data: Vec<u8> = (0..stride * rows as usize)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect();
        let pad = (stride * 8 - columns as usize) as u32;
        data.chunks_mut(stride).for_each(|row| *row.last_mut().unwrap() &= 0xFFu8.checked_shl(pad).unwrap_or(0));
        data
    }

    /// The same stream with its page information (the first segment's data, after its 11-byte
    /// header: width, then height) claiming a page of `width` × `height`.
    fn page_of(data: &[u8], width: u32, height: u32) -> Vec<u8> {
        let mut data = data.to_vec();
        data[11..15].copy_from_slice(&width.to_be_bytes());
        data[15..19].copy_from_slice(&height.to_be_bytes());
        data
    }

    /// Either coding decodes to its samples, black as 0, the bits that pad each row 0: a generic
    /// region of noise, and symbols for letters repeated across the page. With no rows asked for,
    /// nothing is decoded.
    #[test]
    fn both_codings_decode_to_their_samples() {
        let (w, h) = (203u32, 41u32);
        let bits = noise(w, h);
        let generic = encode_jbig2_generic(&bits, w, h).unwrap();
        assert_eq!(decode_jbig2(&generic, None, w, h, usize::MAX).unwrap(), bits);
        assert_eq!(decode_jbig2(b"not JBIG2", None, w, 0, usize::MAX).unwrap(), b"", "no rows asked for");

        // Four lines of three letters, black on white.
        const LETTERS: [[&str; 7]; 3] = [
            ["#####", "#....", "####.", "#....", "#....", "#....", "#####"],
            [".###.", "#...#", "#....", "#....", "#....", "#...#", ".###."],
            ["#...#", "#...#", "#####", "#...#", "#...#", "#...#", "#...#"],
        ];
        let (w, h) = (400u32, 40u32);
        let stride = (w as usize).div_ceil(8);
        let mut marks = vec![0xFFu8; stride * h as usize];
        for (line, top) in [2usize, 12, 22, 32].into_iter().enumerate() {
            for (k, x) in (3..w as usize - 6).step_by(7).enumerate() {
                for (dy, row) in LETTERS[(k + line) % 3].iter().enumerate() {
                    for (dx, _) in row.bytes().enumerate().filter(|(_, c)| *c == b'#') {
                        marks[(top + dy) * stride + (x + dx) / 8] &= !(0x80 >> ((x + dx) % 8));
                    }
                }
            }
        }
        let symbols = encode_jbig2(&marks, w, h).unwrap();
        assert!(symbols.len() < encode_jbig2_generic(&marks, w, h).unwrap().len(), "symbol coded");
        assert_eq!(decode_jbig2(&symbols, None, w, h, usize::MAX).unwrap(), marks);
    }

    /// A region's location is unsigned (ISO/IEC 14492 5.4.4), so one past `i32::MAX` is off the
    /// page. The decoder took it for a signed one: a region at 0xFFFFFFFC came back onto the page
    /// four pixels left of it, and a refinement anywhere past `i32::MAX` was refused, and the page
    /// with it. Each stream is a page of 16 × 12 covered by a generic region, then a region of 8 × 6
    /// of one kind past `i32::MAX`, coded by the standard's procedures, and decodes to that page
    /// alone; moved onto the page, the region draws. Vendored hayro-jbig2 patch 14.
    #[test]
    fn a_region_located_past_i32_max_is_off_the_page() {
        const PAGE: &str = "0000000030000100000013000000100000000c00000000000000004200000000000126000100000035000000100000000c0000000000000000040003fffdff02fefefed906aa2cb6ee4ccb4bc975d02669ca7a9c14c20c638f5a5e1bffac";
        let cases = [
            // At 0xFFFFFFFC, 1.
            ("generic", "00000002260001000000220000000800000006fffffffc00000001000003fffdff02fefefe17c876ae6f77ffac"),
            // At 1, 0xFFFFFFFE, with its symbol dictionary.
            (
                "text",
                "000000020000010000001e000003fffdff02fefefe00000001000000014fdda14149cec477ed5fffac00000003062002010000001b000000080000000600000001fffffffe00001000000001a943ffac",
            ),
            // At 0xFFFFFFFC, 1, with its pattern dictionary.
            (
                "halftone",
                "000000021000010000000a000101000000011fffac00000003162002010000002e0000000800000006fffffffc0000000100000000000800000006000000000000000001000000d09301774485ffac",
            ),
            // At 0xFFFFFFFC, 1, reading the page there (7.4.7.4): 0s.
            ("refinement of the page", "000000022a00010000001e0000000800000006fffffffc000000010400ffffffff0bd98f188428ffac"),
            // Of an intermediate region at the same place (7.4.7.5), 1, 0xFFFFFFFE.
            (
                "refinement of a region",
                "0000000224000100000023000000080000000600000001fffffffe000003fffdff02fefefe3d81322da16b7fffac000000032a2002010000001e000000080000000600000001fffffffe0000ffffffffe29946b82229ffac",
            ),
            // Of an intermediate refinement of an intermediate region, all at 0xFFFFFFFC, 1.
            (
                "refinement of a refinement",
                "00000002240001000000230000000800000006fffffffc00000001000003fffdff02fefefecdfd921ec8c47fffac00000003282002010000001e0000000800000006fffffffc000000010000ffffffff903cdb447a47ffac000000042a2003010000001e0000000800000006fffffffc000000010000ffffffffdcd6078cab86ffac",
            ),
        ];
        let hex = |text: &str| (0..text.len() / 2).map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap()).collect::<Vec<u8>>();
        let decode = |segments: &str| decode_jbig2(&[hex(PAGE), hex(segments)].concat(), None, 16, 12, usize::MAX).unwrap();
        let page = decode("");
        assert!(page.iter().any(|&byte| byte != 0xFF), "the page has ink");
        for (kind, segments) in cases {
            assert_eq!(decode(segments), page, "{kind}");
            // Moved onto the page, to 4, 1 or 1, 2, the same regions draw: they decode, and are
            // left out for where they are.
            let (off, on) = [("fffffffc00000001", "0000000400000001"), ("00000001fffffffe", "0000000100000002")]
                .into_iter()
                .find(|(off, _)| segments.contains(off))
                .unwrap();
            assert_ne!(decode(&segments.replace(off, on)), page, "{kind} on the page");
        }

        // A refinement at 2, 0 of an intermediate region at 0x80000000, 0. A refinement of a
        // region refines it where it is, "GRREFERENCEDX 0, GRREFERENCEDY 0" (Table 35), and the two
        // should be at one place (7.4.7.5). Where they aren't, it decodes as it does where they are.
        const ELSEWHERE: &str = "000000022400010000002300000008000000068000000000000000000003fffdff02fefefe1bca3149568cdfffac000000032a2002010000001e000000080000000600000002000000000000ffffffff784a909cc761ffac";
        let refined = decode(ELSEWHERE);
        assert_eq!(refined, decode(&ELSEWHERE.replace("8000000000000000", "0000000200000000")), "the region at 2, 0 too");
        assert_ne!(refined, page, "drawn");
    }

    /// A refinement of the page refines "the current contents of the page buffer, restricted to the
    /// area of the page buffer specified by this segment's region segment information field"
    /// (ISO/IEC 14492 7.4.7.4), at an offset of 0 (Table 35), and reads 0 outside that area
    /// (6.3.5.2). The decoder read the page around the area too, and refused a refinement wider or
    /// taller than the page. A refinement of another region refines that region, not the page. And
    /// a reference 32 or more left of a refinement (GRREFERENCEDX below −31, as a symbol refined
    /// with an RDX below −31 has, Table 18) is read from where each row starts in it: each row's
    /// first context read 0 for the reference column it starts at, and from 33 left for the one
    /// before it too. Each stream is coded by the standard's procedures and decodes to its
    /// picture, `#` for black. Vendored hayro-jbig2 patch 16.
    #[test]
    fn a_refinement_refines_the_page_in_its_area_and_any_reference_where_it_is() {
        let cases: [(&str, &str, &[&str]); 5] = [
            (
                "a 48 × 6 page a generic region has inked, refined at 33: ink on both sides of the area, template 0",
                "000000003000010000001300000030000000060000000000000000420000000000012600010000004100000030000000060000000000000000040003fffdff02fefefe331433558a5c5056cf412003ec9a38164df39bd4463b9331502c4fb1c267ab77b665aa5962ffac000000022a0001000000210000000c0000000600000021000000000400ffffffff9a6c35bbeee221f687ffac",
                &[
                    ".#..##.######..#.#..#####.##..#........##.##..##",
                    ".#..####.####.#...###.##.##.#.#.....#.#.####.###",
                    "##.#......#..#....#..#.###.#..#.#####.###.######",
                    "......#....###.#.##..#.##........#.#.#...#####..",
                    "#.##..###.#.#.##.....#...#.#..#.##.#..#...#...#.",
                    ".###...#..#..#..##..#.#..#####.#.##.......###.##",
                ],
            ),
            (
                "a 24 × 8 inked page refined at 5, 1, template 1",
                "000000003000010000001300000018000000080000000000000000420000000000012600010000003500000018000000080000000000000000040003fffdff02fefefeb1e133cb8bcbb8510e81d219bb880dfb40a311dc5c298819efffac000000022a00010000001b0000000a0000000500000005000000010401e55c0078dcd30fffac",
                &[
                    "#.##.##...##....##.##..#",
                    "#.##.##...#.####..##..#.",
                    "##..##.##...#...###..#.#",
                    ".....#.#.##.#...###....#",
                    "..#.#...#.#...#.#...##.#",
                    "....######...#.####....#",
                    "..######..##.#.#..##.#..",
                    "...#..##.####.#..##...#.",
                ],
            ),
            (
                "a 16 × 5 inked page refined by a region 4 wider than it, template 0",
                "000000003000010000001300000010000000050000000000000000420000000000012600010000002600000010000000050000000000000000040003fffdff02fefefe22e5e253a429cbf2c701ffac000000022a000100000023000000140000000500000000000000000400ffffffff3afa516532bbe3d481926fffac",
                &[".#.#.#.#..#.#.#.", "##.####..###...#", "..####.#..####.#", ".###.#.....#..#.", "#.###..#########"],
            ),
            (
                "an intermediate region at 4, 1 on an inked 20 × 8 page, refined there (OR): the region, not the page, template 0",
                "000000003000010000001300000014000000080000000000000000420000000000012600010000003000000014000000080000000000000000040003fffdff02fefefeb91e434e1726ca89ed06c2bffb0fe5fa7fccc80fffac00000002240001000000230000000a000000050000000400000001000003fffdff02fefefece7a9826ba81bfffac000000032a2002010000001e0000000a0000000500000004000000010000ffffffffd046ee6f6190ffac",
                &[
                    "#.#.#....#.#...#....",
                    "##..#.#.#.##.....##.",
                    "##.#.#.#.#.###.##...",
                    "###.##.####..###.#.#",
                    "...#.###..###.#.#..#",
                    "#...#######.#...####",
                    "..##.##...#..##..#.#",
                    "#...##...#.##....#..",
                ],
            ),
            (
                "a symbol refined in a symbol dictionary from a 40 × 6 one, RDX −33: 33 left, template 1",
                "0000000030000100000013000000070000000600000000000000004200000000000100000100000036000003fffdff02fefefe00000001000000014f913296209e2187f37bddb8589c8beb7d70109de7124ebe47d04914ef7e655c3c4cffac00000002002001010000001f100203fffdff02fefefe00000001000000014fe451a27bddf49f7ef7e7ffac00000003062002010000001b0000000700000006000000000000000004001000000001a943ffac",
                &[".####.#", ".##..##", "......#", "...#..#", "#.#.###", "....##."],
            ),
        ];
        let hex = |text: &str| (0..text.len() / 2).map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap()).collect::<Vec<u8>>();
        for (form, stream, picture) in cases {
            let (w, h) = (picture[0].len(), picture.len());
            // As the filter gives it: black as 0, each row padded to a byte with 0s.
            let mut rows = vec![0u8; w.div_ceil(8) * h];
            for (y, row) in picture.iter().enumerate() {
                for (x, _) in row.bytes().enumerate().filter(|(_, c)| *c == b'.') {
                    rows[y * w.div_ceil(8) + x / 8] |= 0x80 >> (x % 8);
                }
            }
            assert_eq!(decode_jbig2(&hex(stream), None, w as u32, h as u32, usize::MAX).unwrap(), rows, "{form}");
        }
    }

    /// What a page costs is the same however much a run has left. Given exactly that it decodes,
    /// and less stops it: spending all that's left, and what comes after isn't read. A page that
    /// fails still spends what it took.
    #[test]
    fn a_run_of_images_spends_its_allowance_down_to_the_step() {
        let (w, h) = (203u32, 41u32);
        let data = encode_jbig2_generic(&noise(w, h), w, h).unwrap();
        let whole = decode_jbig2(&data, None, w, h, usize::MAX).unwrap();
        let mut work = u64::MAX;
        assert_eq!(decode_jbig2_within(&data, None, w, h, usize::MAX, &mut work).unwrap(), whole);
        let cost = u64::MAX - work;
        assert!(cost >= u64::from(w) * u64::from(h), "a pixel at a time: {cost}");
        let mut work = cost;
        assert_eq!(decode_jbig2_within(&data, None, w, h, usize::MAX, &mut work).unwrap(), whole, "exactly enough");
        assert_eq!(work, 0);
        let mut work = cost + 7;
        decode_jbig2_within(&data, None, w, h, usize::MAX, &mut work).unwrap();
        assert_eq!(work, 7, "only what it took");
        let mut work = cost - 1;
        assert!(matches!(decode_jbig2_within(&data, None, w, h, usize::MAX, &mut work), Err(FilterError::Spent)), "a step short");
        assert_eq!(work, 0, "spends all that was left");
        // Nothing left: not read at all, so this garbage isn't reported as garbage.
        assert!(matches!(decode_jbig2_within(b"garbage", None, w, h, usize::MAX, &mut work), Err(FilterError::Spent)));
        // Too little even for its page.
        let mut work = 3;
        assert!(decode_jbig2_within(&data, None, w, h, usize::MAX, &mut work).is_err());
        assert_eq!(work, 0);
        // A region that can't be read after the one that can: the page fails having spent the
        // first, and the segment it fails on, and that isn't being out of allowance.
        let bad_region = [0, 0, 0, 9, 38, 0, 1, 0, 0, 0, 1, 0xFF];
        let mut work = u64::MAX;
        assert!(decode_jbig2_within(&[&data[..], &bad_region].concat(), None, w, h, usize::MAX, &mut work).is_err());
        assert_eq!(u64::MAX - work, cost + 1 + 4 * bad_region.len() as u64, "the first region, a segment more, and reading it");
        // Not a stream at all: it pays for being read, and globals count with it.
        let mut work = u64::MAX;
        assert!(matches!(decode_jbig2_within(b"garbage", Some(b"globals"), w, h, usize::MAX, &mut work), Err(FilterError::Corrupt { .. })));
        assert_eq!(u64::MAX - work, 4 * 14);
    }

    /// A refinement whose reference is offset right of it (GRREFERENCEDX above 0, as a refined text
    /// instance has from a wider bitmap, ISO/IEC 14492 Table 12, and a refined symbol from an RDX
    /// above 0, Table 18) reads the reference from its first column. Left of the reference the
    /// decoder emptied what it had read of it, and at its first column didn't read it again, so its
    /// first 32 columns read as 0: such a refinement decoded to other pixels than were coded, or
    /// failed. Each stream is a refinement coded by the standard's procedures on a page of its own
    /// size, and decodes to its picture, `#` for black. Vendored hayro-jbig2 patch 15.
    #[test]
    fn a_refinement_reads_a_reference_right_of_it_from_its_first_column() {
        let cases: [(&str, &str, &[&str]); 4] = [
            (
                "a text instance of an 8 × 6 symbol refined 6 wider: 3 right, template 0",
                "00000000300001000000130000000e000000060000000000000000420000000000010000010000001d000003fffdff02fefefe00000001000000014fdd3dba48bdddde35ffac00000002062001010000002c0000000e000000060000000000000000040012ffffffff00000001a96b4a6b78b7fc094fadde1b3e881fffac",
                &[".#.#.#....#...", "##.###....##.#", ".###...###..#.", "#.......#...#.", "#.####.#...#..", "#.##...###.#.."],
            ),
            (
                "refined 2 wider and 2 taller, RDX and RDY 1: 2 right, 2 down, template 1",
                "00000000300001000000130000000a000000080000000000000000420000000000010000010000001d000003fffdff02fefefe00000001000000014fdd9c52d3ea827a43ffac0000000206200101000000280000000a00000008000000000000000004801200000001a96c00cdac6dd5f6f848ce872411cfffac",
                &["####.##...", "####..####", "####..##..", "...##...#.", "#......#..", "..#.....#.", "##....####", ".#........"],
            ),
            (
                "a symbol refined in a symbol dictionary, RDX 5, RDY −1, template 0 with its AT pixels moved",
                "00000000300001000000130000000c000000060000000000000000420000000000010000010000001e000003fffdff02fefefe00000001000000014fdda13efc4029efc95fffac000000020020010100000027000203fffdff02fefefefeff010100000001000000014fc98c352dd84a906f120ffb49793fffac00000003062002010000001b0000000c00000006000000000000000004001000000001a943ffac",
                &["#.#.##.#.###", "..#.#.....##", "##..#...#...", "..##....##..", "##.#.##.####", ".#..#####..#"],
            ),
            (
                "a text instance of a 40 × 5 symbol refined 2 wider, RDX 1: 2 right, across a word, template 0",
                "00000000300001000000130000002a0000000500000000000000004200000000000100000100000031000003fffdff02fefefe00000001000000015240f590d953ae3f4069dede672090d4d61b377906d3f41aafa4f77107ffac00000002062001010000003c0000002a000000050000000000000000040012ffffffff00000001a96c0b9386d4b140486bdc3673dbd031490e468c7f7813d19068ab04a79cbfffac",
                &[
                    "##.#.#..##.##.#####.#....#.#.#..#..######.",
                    ".#.#..###....#..##.#..#.####.#..#.###...##",
                    ".#.#......#....#####....#...###...#####...",
                    "#...##.##.......#...###.###...##..##.###.#",
                    "..#..###..###..#...#.......##.#........##.",
                ],
            ),
        ];
        let hex = |text: &str| (0..text.len() / 2).map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap()).collect::<Vec<u8>>();
        for (form, stream, picture) in cases {
            let (w, h) = (picture[0].len(), picture.len());
            // As the filter gives it: black as 0, each row padded to a byte with 0s.
            let mut rows = vec![0u8; w.div_ceil(8) * h];
            for (y, row) in picture.iter().enumerate() {
                for (x, _) in row.bytes().enumerate().filter(|(_, c)| *c == b'.') {
                    rows[y * w.div_ceil(8) + x / 8] |= 0x80 >> (x % 8);
                }
            }
            assert_eq!(decode_jbig2(&hex(stream), None, w as u32, h as u32, usize::MAX).unwrap(), rows, "{form}");
        }
    }

    /// The page has to be the image's size, and is bounded before it is decoded, being decoded
    /// whole: within `max` as rows of one bit, and within [`MAX_JBIG2_PIXELS`]. Globals are given
    /// to the decoder: ones it skips decode, ones that aren't JBIG2 are an error, as is damaged
    /// data, which never panics.
    #[test]
    fn what_it_cannot_decode_is_an_error() {
        let (w, h) = (203u32, 41u32);
        let data = encode_jbig2_generic(&noise(w, h), w, h).unwrap();
        for (columns, rows) in [(w - 1, h), (w + 1, h), (w, h - 1), (w, h + 1)] {
            let e = decode_jbig2(&data, None, columns, rows, usize::MAX).unwrap_err();
            assert!(matches!(e, FilterError::Corrupt { .. }), "{columns} × {rows}: {e}");
        }
        let page = 26 * h as usize;
        assert!(matches!(decode_jbig2(&data, None, w, h, page - 1), Err(FilterError::LimitExceeded(m)) if m == page - 1), "a byte short");
        assert_eq!(decode_jbig2(&data, None, w, h, page).unwrap().len(), page, "the page within max");
        // 16,384 × 16,384 is 2^28 pixels, which decodes; a row more is past the cap, here or in the
        // decoder, which has the same one and can refuse it first.
        let large = |rows| decode_jbig2(&page_of(&data, 16_384, rows), None, 16_384, rows, usize::MAX);
        assert!(matches!(large(16_385), Err(FilterError::Unsupported(_) | FilterError::Corrupt { .. })), "{:?}", large(16_385));
        assert!(large(16_384).is_ok(), "{:?}", large(16_384).map(|page| page.len()));

        // An extension segment (ISO/IEC 14492 7.4.14), which a decoder skips: its 11-byte header
        // gives the length of the data after it.
        let skipped = [0, 0, 0, 0, 0x3E, 0, 0, 0, 0, 0, 4, 0, 0, 0, 1];
        let generic = decode_jbig2(&data, None, w, h, usize::MAX).unwrap();
        assert_eq!(decode_jbig2(&data, Some(&skipped), w, h, usize::MAX).unwrap(), generic);
        assert!(matches!(decode_jbig2(&data, Some(b"not JBIG2"), w, h, usize::MAX), Err(FilterError::Corrupt { .. })));
        for end in 0..data.len() {
            let _ = decode_jbig2(&data[..end], None, w, h, usize::MAX);
        }
    }

    /// Pixels past a bitmap's edge are 0 (ISO/IEC 14492 6.3.5.2). A page whose default pixel is 1,
    /// and an intermediate text or halftone region whose default pixel is 1, kept 1s past their
    /// width in their last word of each row, and a refinement, which reads its reference a word at
    /// a time, read them as pixels: of rows not a multiple of 32 wide, it decoded to other pixels
    /// than were coded. Each stream is a refinement coded by the standard's procedures from a
    /// reference read as 0 past its edge, and decodes to the picture given, `#` for black.
    /// Vendored hayro-jbig2 patch 13.
    #[test]
    fn a_refinement_reads_nothing_past_its_references_width() {
        // A page of 7 × 6, then of 33 × 6, whose default pixel is 1, refined whole.
        const PAGE_7: &str = "000000003000010000001300000007000000060000000000000000460000000000012a00010000001e000000070000000600000000000000000400ffffffffe42d7e889ebcffac";
        const PAGE_33: &str = "000000003000010000001300000021000000060000000000000000460000000000012a000100000034000000210000000600000000000000000400ffffffff314a8d598037eb3f915ab3d526ccae915ad6a6a7b814111b124ae60fffac";
        // An intermediate text region of 31 × 6 whose default pixel is 1, then one of 7 × 6 on a
        // page of 40, each refined into the page.
        const TEXT_31: &str = "00000000300001000000130000001f0000000600000000000000004200000000000100000100000030000003fffdff02fefefe00000001000000014f96dceb76c7f1ff585971969cd9f88e538261a6f713be68d77e8e1fffac00000002042001010000001b0000001f00000006000000000000000000029000000001a943ffac000000032a2002010000002f0000001f0000000600000000000000000400ffffffff9ac311688adbc8f570a2f96aef93d332e3b1f4b27b704bffac";
        const TEXT_7: &str = "000000003000010000001300000028000000060000000000000000420000000000010000010000001d000003fffdff02fefefe00000001000000014fe1f79f4a95c5b77fffac00000002042001010000001b0000000700000006000000000000000000029000000001a943ffac000000032a2002010000001e000000070000000600000000000000000400ffffffffa71dae12bd7fffac";
        // An intermediate halftone region of 33 × 6 whose default pixel is 1, refined into the page.
        const HALFTONE_33: &str = "000000003000010000001300000021000000060000000000000000420000000000011000010000000a000101000000011fffac00000002142001010000004100000021000000060000000000000000009000000021000000060000000000000000010000004ea6618f551d48ff24c2c5c98124d449f2adea6fca76e4297fffac000000032a20020100000032000000210000000600000000000000000400ffffffffe87db809393a3251e565e7a749e6470ab37f7aef186ceaf5167bffac";
        let cases: [(&str, &str, [&str; 6]); 5] = [
            ("page 7", PAGE_7, ["###.#.#", ".......", "#.####.", "######.", "#.#...#", ".#...##"]),
            (
                "page 33",
                PAGE_33,
                [
                    ".#..##........###..####....#.##.#",
                    ".#.###.###.##.##..#..#...#.....##",
                    ".....##..#....#.##.######.###..#.",
                    "##...#..##.###..###.##..#.#.#.#.#",
                    ".##.#........##.#.#.....###......",
                    "...#.#####.#..#.###......###..###",
                ],
            ),
            (
                "text 31",
                TEXT_31,
                [
                    "....##....###......#.##.#.#....",
                    ".#.....##.......##..###.#...#..",
                    ".#.#.#.####...#####.####...#.#.",
                    "......#...#...#.##...##...#..#.",
                    ".#..##.###.#.###..####...#...#.",
                    "#...#...#..#.#....#.##.#.....#.",
                ],
            ),
            (
                "text 7 on a page of 40",
                TEXT_7,
                [
                    "......#.................................",
                    ".....##.................................",
                    "##...##.................................",
                    "..#.....................................",
                    "##......................................",
                    "..#.....................................",
                ],
            ),
            (
                "halftone 33",
                HALFTONE_33,
                [
                    "##...#.###.#.###.....#.####..####",
                    "##.##.#...#.##.##.##.#...#.#.###.",
                    "###..#.#....##.###...#..##.##..##",
                    "##..#...###...##..#.####.###.#...",
                    "####.###.###.#..#..#.#.####...###",
                    "#.#..#........#....#.#.##.##..#.#",
                ],
            ),
        ];
        let hex = |text: &str| (0..text.len() / 2).map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap()).collect::<Vec<u8>>();
        for (form, stream, picture) in cases {
            let w = picture[0].len();
            // As the filter gives it: black as 0, each row padded to a byte with 0s.
            let mut rows = vec![0u8; w.div_ceil(8) * picture.len()];
            for (y, row) in picture.iter().enumerate() {
                for (x, _) in row.bytes().enumerate().filter(|(_, c)| *c == b'.') {
                    rows[y * w.div_ceil(8) + x / 8] |= 0x80 >> (x % 8);
                }
            }
            assert_eq!(decode_jbig2(&hex(stream), None, w as u32, 6, usize::MAX).unwrap(), rows, "{form}");
        }
    }
}
