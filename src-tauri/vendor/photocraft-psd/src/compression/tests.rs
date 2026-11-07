use super::*;

#[test]
fn checked_zip_validates_trailer_size_and_padding_budget() {
    let layout = PlaneLayout { planes: 1, width: 3, height: 1, depth: 8, version: Version::Psd };
    for compression in [Compression::Zip, Compression::ZipPrediction] {
        let data = encode_planes(compression, &[1, 2, 3], &layout).unwrap();
        for cut in 0..data.len() {
            assert!(decode_planes(compression, &data[..cut], &layout).is_err(), "cut {cut}");
            assert!(validate_planes(compression, &data[..cut], &layout).is_err());
        }
        let mut corrupt = data;
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(decode_planes(compression, &corrupt, &layout).is_err());
        assert!(validate_planes(compression, &corrupt, &layout).is_err());
    }
    let padded = zip_compress(&[1, 2, 3, 0]);
    assert_eq!(decode_planes(Compression::Zip, &padded, &layout).unwrap(), [1, 2, 3]);
    let bomb = zip_compress(&vec![0; (1 << 20) + 4]);
    assert!(decode_planes(Compression::Zip, &bomb, &layout).is_err());
    assert!(validate_planes(Compression::Zip, &bomb, &layout).is_err());
}

fn layout(planes: usize, width: usize, height: usize, depth: u16, version: Version) -> PlaneLayout {
    PlaneLayout { planes, width, height, depth, version }
}

fn pb_roundtrip(src: &[u8]) -> Vec<u8> {
    let enc = packbits::encode_vec(src);
    let dec = packbits::decode(&enc, src.len()).unwrap();
    assert_eq!(dec, src, "encoded: {enc:?}");
    enc
}

#[test]
fn zip_absurd_declared_size_is_an_error() {
    let z = zip_compress(&[7u8; 64]);
    for expected in [1usize << 40, usize::MAX / 2, usize::MAX] {
        assert!(zip_decompress(&z, expected).is_err(), "expected {expected}");
    }
    // Through decode_planes: a tiny ZIP channel claiming a huge (but capped) plane.
    let l = layout(1, 1 << 16, 1 << 15, 8, Version::Psb);
    assert!(decode_planes(Compression::Zip, &z, &l).is_err());
    assert!(decode_planes(Compression::ZipPrediction, &z, &l).is_err());
    // Past MAX_DECODED_BYTES.
    let l = layout(4, 1 << 20, 1 << 20, 32, Version::Psb);
    assert!(decode_planes(Compression::Zip, &z, &l).is_err());
    // Garbage that is not zlib at all.
    assert!(zip_decompress(&[0xff; 16], 1 << 30).is_err());
}

#[test]
fn zip_decompress_exact_and_truncated() {
    let src: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    let z = zip_compress(&src);
    assert_eq!(zip_decompress(&z, src.len()).unwrap(), src);
    // Asking for fewer bytes returns exactly that many.
    assert_eq!(zip_decompress(&z, 1000).unwrap(), &src[..1000]);
    assert!(zip_decompress(&z, src.len() + 1).is_err());
    assert!(zip_decompress(&z, 0).unwrap().is_empty());
}

#[test]
fn packbits_empty() {
    assert!(pb_roundtrip(&[]).is_empty());
}

#[test]
fn packbits_single() {
    assert_eq!(pb_roundtrip(&[7]), vec![0, 7]);
}

#[test]
fn packbits_run_of_two() {
    assert_eq!(pb_roundtrip(&[5, 5]), vec![0xff, 5]);
}

#[test]
fn packbits_run_128() {
    let src = vec![9u8; 128];
    assert_eq!(pb_roundtrip(&src), vec![0x81, 9]);
}

#[test]
fn packbits_run_129() {
    let src = vec![9u8; 129];
    assert_eq!(pb_roundtrip(&src), vec![0x81, 9, 0, 9]);
}

#[test]
fn packbits_run_130_and_256() {
    pb_roundtrip(&[1u8; 130]);
    let enc = pb_roundtrip(&[1u8; 256]);
    assert_eq!(enc, vec![0x81, 1, 0x81, 1]);
}

#[test]
fn packbits_literal_128_and_129() {
    let src: Vec<u8> = (0..128u32).map(|i| i as u8).collect();
    let enc = pb_roundtrip(&src);
    assert_eq!(enc[0], 127);
    assert_eq!(enc.len(), 129);
    let src: Vec<u8> = (0..129u32).map(|i| i as u8).collect();
    let enc = pb_roundtrip(&src);
    assert_eq!(enc[0], 127);
    assert_eq!(enc[129], 0);
    assert_eq!(enc.len(), 131);
}

#[test]
fn packbits_literal_run_boundary() {
    pb_roundtrip(&[1, 2, 3, 3, 3, 3, 4, 5]);
    pb_roundtrip(&[1, 1, 2, 2, 3, 3]);
    pb_roundtrip(&[1, 2, 2, 3]);
    pb_roundtrip(&[0, 0, 0, 1, 0, 0, 0]);
    let enc = pb_roundtrip(&[1, 2, 3, 3, 3]);
    assert_eq!(enc, vec![1, 1, 2, 0xfe, 3]);
}

#[test]
fn packbits_never_emits_noop() {
    for n in 0..300usize {
        let src: Vec<u8> = (0..n).map(|i| ((i / 3) % 5) as u8).collect();
        let enc = pb_roundtrip(&src);
        // Walk the packets to find headers.
        let mut i = 0;
        while i < enc.len() {
            let h = enc[i] as i8;
            assert_ne!(h, -128);
            i += if h >= 0 { h as usize + 2 } else { 2 };
        }
    }
}

#[test]
fn packbits_decode_noop_skipped() {
    assert_eq!(packbits::decode(&[0x80, 0, 42], 1).unwrap(), vec![42]);
}

#[test]
fn packbits_decode_errors() {
    assert!(packbits::decode(&[], 1).is_err());
    assert!(packbits::decode(&[2, 1, 2], 3).is_err()); // literal truncated
    assert!(packbits::decode(&[0xfe], 3).is_err()); // run truncated
    assert!(packbits::decode(&[0xfd, 1], 3).is_err()); // run overflows (4 > 3)
    assert!(packbits::decode(&[3, 1, 2, 3, 4], 3).is_err()); // literal overflows
}

/// `out.len() + expected` past `usize::MAX` used to panic in debug builds and, in release,
/// wrap and return `Ok` having decoded nothing.
#[test]
fn packbits_decode_into_output_length_overflow_is_an_error() {
    let mut out = vec![7u8];
    let r = packbits::decode_into(&[0x00, 0xaa], usize::MAX, &mut out);
    assert!(matches!(r, Err(PsdError::LimitExceeded(_))), "{r:?}");
    assert_eq!(out, [7]);
    // The largest `expected` that fits still decodes up to the input, then reports it short.
    let r = packbits::decode_into(&[0x00, 0xaa], usize::MAX - 1, &mut out);
    assert!(matches!(r, Err(PsdError::Decompress(_))), "{r:?}");
    assert_eq!(out, [7, 0xaa]);
}

#[test]
fn packbits_zero_expected_ignores_input() {
    assert_eq!(packbits::decode(&[0xff, 1], 0).unwrap(), Vec::<u8>::new());
}

#[test]
fn compression_values() {
    for v in 0..6 {
        assert_eq!(Compression::from_u16(v).as_u16(), v);
    }
    assert_eq!(Compression::from_u16(9), Compression::Unknown(9));
}

fn sample_data(l: &PlaneLayout) -> Vec<u8> {
    let n = l.decoded_len().unwrap();
    (0..n).map(|i| ((i * 7 + i / 5) % 251) as u8).collect()
}

#[test]
fn all_codecs_roundtrip_all_depths() {
    for version in [Version::Psd, Version::Psb] {
        for depth in [1u16, 8, 16, 32] {
            for (w, h) in [(0, 0), (1, 1), (3, 2), (17, 5), (0, 3), (4, 0)] {
                for planes in [1, 3] {
                    let l = layout(planes, w, h, depth, version);
                    let data = sample_data(&l);
                    for c in Compression::ALL {
                        let enc = encode_planes(c, &data, &l).unwrap();
                        validate_planes(c, &enc, &l).unwrap();
                        let dec = decode_planes(c, &enc, &l).unwrap();
                        assert_eq!(dec, data, "{c:?} {depth} {w}x{h} {version:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn rle_count_width_psd_vs_psb() {
    let data = vec![0u8; 4];
    let psd = encode_planes(Compression::Rle, &data, &layout(1, 2, 2, 8, Version::Psd)).unwrap();
    let psb = encode_planes(Compression::Rle, &data, &layout(1, 2, 2, 8, Version::Psb)).unwrap();
    assert_eq!(psd, vec![0, 2, 0, 2, 0xff, 0, 0xff, 0]);
    assert_eq!(psb, vec![0, 0, 0, 2, 0, 0, 0, 2, 0xff, 0, 0xff, 0]);
    assert!(decode_planes(Compression::Rle, &psd, &layout(1, 2, 2, 8, Version::Psb)).is_err());
}

#[test]
fn rle_odd_width() {
    let l = layout(1, 7, 3, 16, Version::Psd);
    let data = sample_data(&l);
    let enc = encode_planes(Compression::Rle, &data, &l).unwrap();
    assert_eq!(decode_planes(Compression::Rle, &enc, &l).unwrap(), data);
}

#[test]
fn rle_truncated_fails() {
    let l = layout(1, 10, 10, 8, Version::Psd);
    let data = sample_data(&l);
    let enc = encode_planes(Compression::Rle, &data, &l).unwrap();
    for cut in 0..enc.len() {
        assert!(decode_planes(Compression::Rle, &enc[..cut], &l).is_err(), "cut {cut}");
        assert!(validate_planes(Compression::Rle, &enc[..cut], &l).is_err(), "cut {cut}");
    }
}

#[test]
fn zip_truncated_fails() {
    let l = layout(1, 10, 10, 8, Version::Psd);
    let data = sample_data(&l);
    let enc = encode_planes(Compression::Zip, &data, &l).unwrap();
    for cut in 0..enc.len() - 4 {
        assert!(decode_planes(Compression::Zip, &enc[..cut], &l).is_err(), "cut {cut}");
        assert!(validate_planes(Compression::Zip, &enc[..cut], &l).is_err(), "cut {cut}");
    }
}

/// #2926: a ZIP stream declaring more than `MAX_DECODED_BYTES` is not inflated at parse
/// time. Garbage zlib data proves it: decompressing it would fail, skipping it passes, and
/// decoding the layout still reports the size limit.
#[test]
fn zip_over_budget_is_not_decompressed_at_parse() {
    let l = layout(4, 300_000, 300_000, 8, Version::Psb);
    assert!(l.total_bytes().unwrap() > MAX_DECODED_BYTES);
    let garbage = [0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03];
    for c in [Compression::Zip, Compression::ZipPrediction] {
        validate_planes(c, &garbage, &l).unwrap();
        assert!(matches!(decode_planes(c, &garbage, &l), Err(PsdError::LimitExceeded(_))), "{c:?}");
    }
    // Within the budget the stream is still checked.
    let small = layout(1, 4, 4, 8, Version::Psd);
    assert!(validate_planes(Compression::Zip, &garbage, &small).is_err());
}

#[test]
fn raw_short_fails() {
    let l = layout(2, 3, 3, 8, Version::Psd);
    assert!(decode_planes(Compression::Raw, &[0; 17], &l).is_err());
    assert!(decode_planes(Compression::Raw, &[0; 18], &l).is_ok());
}

#[test]
fn unknown_compression_errors() {
    let l = layout(1, 1, 1, 8, Version::Psd);
    assert!(decode_planes(Compression::Unknown(7), &[0], &l).is_err());
    assert!(encode_planes(Compression::Unknown(7), &[0], &l).is_err());
}

#[test]
fn encode_rejects_wrong_size() {
    let l = layout(1, 2, 2, 8, Version::Psd);
    assert!(encode_planes(Compression::Raw, &[0; 3], &l).is_err());
}

#[test]
fn decoded_len_limits() {
    let l = layout(56, 300_000, 300_000, 32, Version::Psb);
    assert!(matches!(l.decoded_len(), Err(PsdError::LimitExceeded(_))));
    assert!(decode_planes(Compression::Rle, &[0; 16], &l).is_err());
}

/// Real PSBs go past 2 GiB in one decode (#375): a 30000² RGB merged image and a 33000²
/// 16-bit layer channel. Only checks the limit, so nothing is allocated.
#[test]
#[cfg(target_pointer_width = "64")]
fn decoded_len_allows_large_psb_planes() {
    assert_eq!(layout(3, 30_000, 30_000, 8, Version::Psb).decoded_len().unwrap(), 2_700_000_000);
    assert_eq!(layout(1, 33_000, 33_000, 16, Version::Psb).decoded_len().unwrap(), 2_178_000_000);
    assert!(layout(4, 40_000, 40_000, 16, Version::Psb).decoded_len().is_err());
}

#[test]
fn prediction_8bit_known_vector() {
    let l = layout(1, 4, 1, 8, Version::Psd);
    let mut v = vec![10, 12, 11, 255];
    predict(&mut v, &l).unwrap();
    assert_eq!(v, vec![10, 2, 255, 244]);
    unpredict(&mut v, &l).unwrap();
    assert_eq!(v, vec![10, 12, 11, 255]);
}

#[test]
fn prediction_16bit_known_vector() {
    let l = layout(1, 3, 1, 16, Version::Psd);
    let mut v = vec![0x01, 0x00, 0x01, 0x05, 0x00, 0x00];
    predict(&mut v, &l).unwrap();
    assert_eq!(v, vec![0x01, 0x00, 0x00, 0x05, 0xfe, 0xfb]);
    unpredict(&mut v, &l).unwrap();
    assert_eq!(v, vec![0x01, 0x00, 0x01, 0x05, 0x00, 0x00]);
}

#[test]
fn prediction_32bit_plane_shuffle() {
    // Two samples: AABBCCDD and 11223344 → planes [AA,11][BB,22][CC,33][DD,44]
    let l = layout(1, 2, 1, 32, Version::Psd);
    let orig = vec![0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44];
    let mut v = orig.clone();
    predict(&mut v, &l).unwrap();
    let planes = [0xAAu8, 0x11, 0xBB, 0x22, 0xCC, 0x33, 0xDD, 0x44];
    let mut expect = planes;
    for i in (1..8).rev() {
        expect[i] = expect[i].wrapping_sub(expect[i - 1]);
    }
    assert_eq!(v, expect);
    unpredict(&mut v, &l).unwrap();
    assert_eq!(v, orig);
}

#[test]
fn prediction_32bit_floats_roundtrip() {
    let l = layout(1, 5, 2, 32, Version::Psd);
    let mut v = Vec::new();
    for i in 0..10 {
        v.extend_from_slice(&(i as f32 * 0.37 - 1.0).to_be_bytes());
    }
    let enc = encode_planes(Compression::ZipPrediction, &v, &l).unwrap();
    assert_eq!(decode_planes(Compression::ZipPrediction, &enc, &l).unwrap(), v);
}

#[test]
fn zip_bomb_is_bounded() {
    let big = zip_compress(&vec![0u8; 1 << 20]);
    let out = zip_decompress(&big, 10).unwrap();
    assert_eq!(out.len(), 10);
}

#[test]
fn zip_garbage_errors() {
    assert!(zip_decompress(&[1, 2, 3, 4], 4).is_err());
}
