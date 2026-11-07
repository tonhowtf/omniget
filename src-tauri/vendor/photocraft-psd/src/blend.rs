//! Layer blend modes (the 4-character keys stored after the `8BIM` signature).

/// Photoshop blend mode keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[allow(missing_docs)]
pub enum BlendMode {
    PassThrough,
    #[default]
    Normal,
    Dissolve,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    DarkerColor,
    Lighten,
    Screen,
    ColorDodge,
    LinearDodge,
    LighterColor,
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
    /// A key not known to this crate, preserved verbatim.
    Unknown([u8; 4]),
}

impl BlendMode {
    /// All known modes (27 layer modes plus pass-through for groups).
    pub const ALL: [BlendMode; 28] = [
        BlendMode::PassThrough,
        BlendMode::Normal,
        BlendMode::Dissolve,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::ColorBurn,
        BlendMode::LinearBurn,
        BlendMode::DarkerColor,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::ColorDodge,
        BlendMode::LinearDodge,
        BlendMode::LighterColor,
        BlendMode::Overlay,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::VividLight,
        BlendMode::LinearLight,
        BlendMode::PinLight,
        BlendMode::HardMix,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Subtract,
        BlendMode::Divide,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];

    /// The stored 4-byte key.
    pub fn key(self) -> [u8; 4] {
        *match self {
            BlendMode::PassThrough => b"pass",
            BlendMode::Normal => b"norm",
            BlendMode::Dissolve => b"diss",
            BlendMode::Darken => b"dark",
            BlendMode::Multiply => b"mul ",
            BlendMode::ColorBurn => b"idiv",
            BlendMode::LinearBurn => b"lbrn",
            BlendMode::DarkerColor => b"dkCl",
            BlendMode::Lighten => b"lite",
            BlendMode::Screen => b"scrn",
            BlendMode::ColorDodge => b"div ",
            BlendMode::LinearDodge => b"lddg",
            BlendMode::LighterColor => b"lgCl",
            BlendMode::Overlay => b"over",
            BlendMode::SoftLight => b"sLit",
            BlendMode::HardLight => b"hLit",
            BlendMode::VividLight => b"vLit",
            BlendMode::LinearLight => b"lLit",
            BlendMode::PinLight => b"pLit",
            BlendMode::HardMix => b"hMix",
            BlendMode::Difference => b"diff",
            BlendMode::Exclusion => b"smud",
            BlendMode::Subtract => b"fsub",
            BlendMode::Divide => b"fdiv",
            BlendMode::Hue => b"hue ",
            BlendMode::Saturation => b"sat ",
            BlendMode::Color => b"colr",
            BlendMode::Luminosity => b"lum ",
            BlendMode::Unknown(ref k) => k,
        }
    }

    /// Parses a stored key; unknown keys map to [`BlendMode::Unknown`].
    pub fn from_key(key: [u8; 4]) -> Self {
        BlendMode::ALL.iter().copied().find(|m| m.key() == key).unwrap_or(BlendMode::Unknown(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_keys_roundtrip_and_unique() {
        let mut keys = std::collections::HashSet::new();
        for m in BlendMode::ALL {
            assert_eq!(BlendMode::from_key(m.key()), m);
            assert!(keys.insert(m.key()));
        }
        assert_eq!(keys.len(), 28);
    }

    #[test]
    fn unknown_preserved() {
        let m = BlendMode::from_key(*b"zzzz");
        assert_eq!(m, BlendMode::Unknown(*b"zzzz"));
        assert_eq!(m.key(), *b"zzzz");
    }

    #[test]
    fn specific_keys() {
        assert_eq!(BlendMode::Multiply.key(), *b"mul ");
        assert_eq!(BlendMode::from_key(*b"smud"), BlendMode::Exclusion);
        assert_eq!(BlendMode::from_key(*b"idiv"), BlendMode::ColorBurn);
        assert_eq!(BlendMode::default(), BlendMode::Normal);
    }
}
