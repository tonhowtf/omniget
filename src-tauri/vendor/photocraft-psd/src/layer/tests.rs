use super::*;

fn sample_record(version: Version) -> LayerRecord {
    let rect = Rect::from_xywh(-3, 2, 5, 4);
    let px: Vec<u8> = (0..20).collect();
    let mut channels = Vec::new();
    for (id, comp) in [(-1, Compression::Rle), (0, Compression::Raw), (1, Compression::Zip), (2, Compression::ZipPrediction)] {
        channels.push(ChannelData::encode(id, comp, &px, 5, 4, 8, version).unwrap());
    }
    let mask = LayerMask::new(Rect::from_xywh(0, 0, 2, 2), 255, LayerMask::FLAG_RELATIVE);
    channels.push(ChannelData::encode(-2, Compression::Rle, &[1, 2, 3, 4], 2, 2, 8, version).unwrap());
    LayerRecord {
        rect,
        channels,
        blend_mode: BlendMode::Screen,
        opacity: 200,
        clipping: 1,
        flags: LayerFlags(LayerFlags::HIDDEN),
        mask: MaskData::Mask(mask),
        blending_ranges: BlendingRanges::full(3),
        name: b"abc".to_vec(),
        blocks: vec![TaggedBlock::unicode_name("\u{e4}bc"), TaggedBlock::layer_id(5)],
        ..Default::default()
    }
}

fn rt(info: &LayerInfo, version: Version) -> Vec<u8> {
    let mut out = Vec::new();
    info.write_body(&mut out, version).unwrap();
    let back = LayerInfo::read_body(&out, version).unwrap();
    assert_eq!(&back, info);
    let mut again = Vec::new();
    back.write_body(&mut again, version).unwrap();
    assert_eq!(again, out);
    out
}

#[test]
fn empty_layer_info() {
    let out = rt(&LayerInfo::default(), Version::Psd);
    assert_eq!(out, vec![0, 0]);
}

#[test]
fn record_roundtrip_psd_psb() {
    for v in [Version::Psd, Version::Psb] {
        let info = LayerInfo { merged_alpha: true, layers: vec![sample_record(v), LayerRecord::default()], padding: None };
        let out = rt(&info, v);
        assert_eq!(out.len() % 2, 0);
        assert_eq!(&out[..2], &(-2i16).to_be_bytes());
    }
}

#[test]
fn psb_channel_lengths_are_8_bytes() {
    let mut rec = LayerRecord::default();
    rec.channels.push(ChannelData::new(0, Some(Compression::Raw), vec![]));
    let info = LayerInfo { layers: vec![rec], ..Default::default() };
    let psd = rt(&info, Version::Psd);
    let psb = rt(&info, Version::Psb);
    assert_eq!(psb.len(), psd.len() + 4);
}

#[test]
fn channel_decode_via_record() {
    let rec = sample_record(Version::Psd);
    let px: Vec<u8> = (0..20).collect();
    for id in [-1, 0, 1, 2] {
        assert_eq!(rec.decode_channel(id, 8, Version::Psd).unwrap(), px);
    }
    assert_eq!(rec.decode_channel(-2, 8, Version::Psd).unwrap(), vec![1, 2, 3, 4]);
    assert!(rec.decode_channel(7, 8, Version::Psd).is_err());
}

#[test]
fn record_accessors() {
    let rec = sample_record(Version::Psd);
    assert_eq!(rec.name(), "\u{e4}bc");
    assert!(!rec.is_visible());
    assert_eq!(rec.layer_id(), Some(5));
    assert_eq!(rec.fill_opacity(), 255);
    assert_eq!(rec.section_type(), SectionType::Other);
    assert!(rec.layer_mask().is_some());
    assert_eq!(rec.channel_rect(-2), Rect::from_xywh(0, 0, 2, 2));
    assert_eq!(rec.channel_rect(0), rec.rect);
    let mut plain = LayerRecord { name: b"legacy".to_vec(), ..Default::default() };
    assert_eq!(plain.name(), "legacy");
    plain.blocks.push(TaggedBlock::fill_opacity(10));
    assert_eq!(plain.fill_opacity(), 10);
    plain.block_mut(b"iOpa").unwrap().data[0] = 11;
    assert_eq!(plain.fill_opacity(), 11);
}

#[test]
fn mask_variants_roundtrip() {
    let base = LayerMask::new(Rect::from_xywh(1, 2, 3, 4), 0, 0);
    let with_real = LayerMask { real: Some(RealMask { flags: 1, background: 255, rect: Rect::from_xywh(0, 0, 9, 9) }), trailing: vec![], ..base.clone() };
    let with_params = LayerMask {
        flags: LayerMask::FLAG_PARAMETERS,
        parameters: Some(MaskParameters {
            flags: 0b1111,
            user_density: Some(128),
            user_feather: Some(2.5),
            vector_density: Some(64),
            vector_feather: Some(0.5),
        }),
        real: Some(RealMask::default()),
        trailing: vec![0, 0],
        ..base.clone()
    };
    let partial_params = LayerMask {
        flags: LayerMask::FLAG_PARAMETERS,
        parameters: Some(MaskParameters { flags: 0b0010, user_feather: Some(1.0), ..Default::default() }),
        trailing: vec![],
        ..base.clone()
    };
    for m in [base, with_real, with_params, partial_params] {
        let mut out = Vec::new();
        m.write(&mut out);
        assert_eq!(LayerMask::parse(&out).unwrap(), m);
    }
}

/// Photoshop stores the real-mask fields before the mask parameters (psd-tools
/// mask-density-layervectormask.psd): user + vector mask with both densities set.
#[test]
fn real_mask_before_parameters_photoshop_layout() {
    let mut d = Vec::new();
    Rect::from_xywh(15, -1, 18, 10).write(&mut d);
    d.extend([0, LayerMask::FLAG_PARAMETERS | 8]);
    d.extend([0, 255]);
    Rect::from_xywh(0, 0, 32, 8).write(&mut d);
    d.extend([0b0101, 64, 64, 0]);
    let m = LayerMask::parse(&d).unwrap();
    assert_eq!(m.real.map(|r| r.rect), Some(Rect::from_xywh(0, 0, 32, 8)));
    assert_eq!(m.real.map(|r| r.background), Some(255));
    let p = m.parameters.unwrap();
    assert_eq!((p.user_density, p.vector_density), (Some(64), Some(64)));
    assert_eq!(m.trailing, vec![0]);
    assert!(m.real_first);
    let mut out = Vec::new();
    m.write(&mut out);
    assert_eq!(out, d);
}

/// A density-only record in the spec's order (#949): the parameter flags byte at 18 says one
/// value byte, so the spec's reading excludes the 38-byte length only if the real mask is
/// ignored, while byte 36 (inside the real rectangle's `right`) is 0 and makes Photoshop's
/// reading fit too. Photoshop's reading puts the density (64) where a 0-or-255 background
/// belongs, so the spec's order wins and the density survives.
#[test]
fn density_only_spec_layout_is_not_read_as_photoshop_order() {
    for (density, right) in [(64u8, 1u32), (200, 1), (64, 300), (1, 7)] {
        let mut d = Vec::new();
        Rect::from_xywh(0, 0, 1, 1).write(&mut d);
        d.extend([255, LayerMask::FLAG_PARAMETERS]);
        d.extend([0b0001, density]);
        d.extend([0, 0]);
        Rect::from_xywh(0, 0, right, 1).write(&mut d);
        let m = LayerMask::parse(&d).unwrap();
        assert!(!m.real_first, "density {density}, right {right}: {m:?}");
        assert_eq!(m.parameters.unwrap().user_density, Some(density));
        assert_eq!(m.real, Some(RealMask { flags: 0, background: 0, rect: Rect::from_xywh(0, 0, right, 1) }));
        assert!(m.trailing.is_empty());
        let mut out = Vec::new();
        m.write(&mut out);
        assert_eq!(out, d);
    }
    // Photoshop's order with the same lengths still reads as Photoshop's order.
    let mut d = Vec::new();
    Rect::from_xywh(0, 0, 1, 1).write(&mut d);
    d.extend([255, LayerMask::FLAG_PARAMETERS]);
    d.extend([0, 255]);
    Rect::from_xywh(0, 0, 1, 1).write(&mut d);
    d.extend([0b0001, 64]);
    let m = LayerMask::parse(&d).unwrap();
    assert!(m.real_first, "{m:?}");
    assert_eq!(m.parameters.unwrap().user_density, Some(64));
    assert_eq!(m.real.map(|r| r.background), Some(255));
    let mut out = Vec::new();
    m.write(&mut out);
    assert_eq!(out, d);
}

/// The spec's order (parameters, then real fields) still parses and stays byte-stable.
#[test]
fn real_mask_after_parameters_spec_layout() {
    let mut d = Vec::new();
    Rect::from_xywh(1, 2, 3, 4).write(&mut d);
    d.extend([255, LayerMask::FLAG_PARAMETERS]);
    d.extend([0b0011, 200]);
    d.extend(1.5f64.to_be_bytes());
    d.extend([0, 255]);
    Rect::from_xywh(0, 0, 3, 3).write(&mut d);
    let m = LayerMask::parse(&d).unwrap();
    assert!(!m.real_first);
    assert_eq!(m.real.map(|r| r.rect), Some(Rect::from_xywh(0, 0, 3, 3)));
    assert_eq!(m.parameters.unwrap().user_feather, Some(1.5));
    let mut out = Vec::new();
    m.write(&mut out);
    assert_eq!(out, d);
    // Parameters alone (no real mask) keep reading in the spec's order.
    let mut d = Vec::new();
    Rect::from_xywh(1, 2, 3, 4).write(&mut d);
    d.extend([255, LayerMask::FLAG_PARAMETERS, 0b1010]);
    d.extend(1.0f64.to_be_bytes());
    d.extend(2.0f64.to_be_bytes());
    d.extend([0, 0]);
    let m = LayerMask::parse(&d).unwrap();
    assert_eq!(m.real, None);
    assert_eq!(m.parameters.map(|p| (p.user_feather, p.vector_feather)), Some((Some(1.0), Some(2.0))));
}

#[test]
fn mask_sizes() {
    let mut out = Vec::new();
    LayerMask::new(Rect::default(), 0, 0).write(&mut out);
    assert_eq!(out.len(), 20);
    let m = LayerMask { real: Some(RealMask::default()), trailing: vec![], ..LayerMask::new(Rect::default(), 0, 0) };
    let mut out = Vec::new();
    m.write(&mut out);
    assert_eq!(out.len(), 36);
}

#[test]
fn short_mask_kept_raw() {
    let mut rec = LayerRecord { mask: MaskData::Raw(vec![1, 2, 3]), ..Default::default() };
    rec.channels.clear();
    let info = LayerInfo { layers: vec![rec], ..Default::default() };
    rt(&info, Version::Psd);
}

#[test]
fn flags_helpers() {
    let mut f = LayerFlags::default();
    f.set_hidden(true);
    f.set_transparency_protected(true);
    assert!(f.hidden() && f.transparency_protected());
    f.set_pixel_data_irrelevant(true);
    assert!(f.bit4_useful() && f.pixel_data_irrelevant());
    f.set_hidden(false);
    assert!(!f.hidden());
}

#[test]
fn blending_ranges_parse() {
    let br = BlendingRanges::full(3);
    assert_eq!(br.data.len(), 32);
    let r = br.ranges();
    assert_eq!(r.len(), 4);
    assert_eq!(r[0].source, [0, 0, 255, 255]);
}

#[test]
fn rect_helpers() {
    let r = Rect::from_xywh(-5, -6, 10, 3);
    assert_eq!((r.width(), r.height()), (10, 3));
    assert_eq!(r.size().unwrap(), (10, 3));
    assert!(!r.is_empty());
    let bad = Rect { top: 5, left: 0, bottom: 0, right: 1 };
    assert!(bad.size().is_err());
    assert!(bad.is_empty());
    let huge = Rect { top: 0, left: i32::MIN, bottom: 1, right: i32::MAX };
    assert!(huge.size().is_err());
}

#[test]
fn truncation_errors() {
    let info = LayerInfo { layers: vec![sample_record(Version::Psd)], ..Default::default() };
    let mut out = Vec::new();
    info.write_body(&mut out, Version::Psd).unwrap();
    // The final byte may be the (optional) even padding; stop before it.
    for cut in 0..out.len() - 1 {
        assert!(LayerInfo::read_body(&out[..cut], Version::Psd).is_err(), "cut {cut}");
    }
}

#[test]
fn bad_blend_signature() {
    let info = LayerInfo { layers: vec![LayerRecord::default()], ..Default::default() };
    let mut out = Vec::new();
    info.write_body(&mut out, Version::Psd).unwrap();
    // count(2) + rect(16) + nch(2) = 20 → signature
    out[20] = b'X';
    assert!(matches!(LayerInfo::read_body(&out, Version::Psd), Err(PsdError::InvalidSignature { .. })));
}

#[test]
fn too_many_layers_rejected() {
    let out = 1000i16.to_be_bytes();
    assert!(LayerInfo::read_body(&out, Version::Psd).is_err());
}

#[test]
fn zero_length_channel() {
    let mut rec = LayerRecord::default();
    rec.channels.push(ChannelData::new(0, None, vec![]));
    let info = LayerInfo { layers: vec![rec], ..Default::default() };
    rt(&info, Version::Psd);
    assert_eq!(info.layers[0].decode_channel(0, 8, Version::Psd).unwrap(), Vec::<u8>::new());
}

/// The computed length matches the bytes written for every mask form, long names, custom block
/// padding, long-length blocks and trailing bytes.
#[test]
fn unpadded_len_matches_the_written_body() {
    for version in [Version::Psd, Version::Psb] {
        let mut layers = vec![sample_record(version), LayerRecord::default()];
        for (params, real, real_first) in [(true, true, true), (true, true, false), (true, false, true), (false, true, true)] {
            let mut mask = LayerMask::new(Rect::from_xywh(1, 2, 3, 4), 0, LayerMask::FLAG_PARAMETERS);
            mask.parameters =
                params.then_some(MaskParameters { flags: 0x0f, user_density: Some(1), user_feather: Some(2.5), vector_density: Some(3), vector_feather: None });
            mask.real = real.then_some(RealMask { flags: 0, background: 255, rect: Rect::from_xywh(0, 0, 1, 1) });
            mask.real_first = real_first;
            mask.trailing = vec![0; 3];
            let mut rec = sample_record(version);
            rec.mask = MaskData::Mask(mask);
            rec.name = vec![b'n'; 300];
            rec.blocks.push(TaggedBlock { signature: *b"8BIM", key: *b"lsct", data: vec![1; 5], padding: Some(vec![0; 3]) });
            rec.blocks.push(TaggedBlock { signature: *b"8BIM", key: *b"Lr16", data: vec![2; 7], padding: None });
            rec.extra_trailing = vec![9; 2];
            layers.push(rec);
        }
        layers.push(LayerRecord { mask: MaskData::Raw(vec![5; 11]), name: b"ab".to_vec(), ..Default::default() });
        let info = LayerInfo { layers, ..Default::default() };
        let mut out = Vec::new();
        info.write_unpadded(&mut out, version).unwrap();
        assert_eq!(info.unpadded_len(version).unwrap(), out.len() as u64, "{version:?}");
    }
}

/// Padding without cloning the layers writes the same bytes as `pad_to` on a clone, and the
/// channel data length hint is exact.
#[test]
fn unpadded_write_matches_pad_to_on_a_clone() {
    use crate::{ColorMode, Compression, testgen};
    for version in [Version::Psd, Version::Psb] {
        for depth in [8u16, 16, 32] {
            for compression in [Compression::Raw, Compression::Rle] {
                let file = testgen::layered(version, ColorMode::Rgb, depth, compression);
                let mut info = file.layer_info.clone().unwrap();
                for align in [1u64, 2, 4, 8] {
                    let mut cloned = info.clone();
                    cloned.pad_to(version, align).unwrap();
                    let mut want = Vec::new();
                    cloned.write_body(&mut want, version).unwrap();
                    let pad = info.pad_len(version, align).unwrap();
                    let mut got = Vec::new();
                    info.write_unpadded(&mut got, version).unwrap();
                    assert!(got.len() >= info.channel_data_len());
                    got.resize(got.len() + pad, 0);
                    assert_eq!(got, want, "{version:?} {depth} {compression:?} align {align}");
                }
                info.padding = Some(vec![7, 7]);
                let mut with_padding = Vec::new();
                info.write_body(&mut with_padding, version).unwrap();
                assert!(with_padding.ends_with(&[7, 7]));
            }
        }
    }
}

#[test]
fn nondefault_padding_kept() {
    let info = LayerInfo { padding: Some(vec![0, 0]), ..Default::default() };
    let out = rt(&info, Version::Psd);
    assert_eq!(out.len(), 4);
}

#[test]
fn unknown_blend_and_blocks_passthrough() {
    let rec = LayerRecord { blend_mode: BlendMode::Unknown(*b"wxyz"), blocks: vec![TaggedBlock::new(*b"Zzzz", vec![1, 2, 3, 4, 5])], ..Default::default() };
    rt(&LayerInfo { layers: vec![rec], ..Default::default() }, Version::Psd);
}
