//! Residents and avatars of the 3D city (GameChangeLLM M4, estudos 02, 03, 13).
//!
//! - [`genome`]: the 32-byte appearance genome v1 (`encode`, `decode`, `random`).
//! - [`catalog`]: the versioned appearance catalog, shared with the frontend.
//! - [`traits`]: closed-vocabulary traits (`{attributes:[{trait_type, value}]}`
//!   plus phenotype words) to genome.
//! - [`identity`]: who a resident is, deterministic generator and the JSON
//!   Schema an LLM fills.
//! - [`store`]: the local avatar record.
//!
//! The body geometry is Anny (Apache-2.0) with MakeHuman data (CC0), baked by
//! `tools/bake_anny.py`. Clothing and hair are procedural for now; MakeHuman
//! MHCLO garments (CC0) come in later as catalog pieces with their licenses.

pub mod catalog;
pub mod genome;
pub mod identity;
pub mod rng;
pub mod store;
pub mod traits;

pub use catalog::{catalog, catalog_hash, Catalog};
pub use genome::{Genome, GenomeError, GENOME_LEN};
pub use identity::{identity_prompt_schema, npc_identities, Identity, Resident};
pub use traits::from_traits;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genome_roundtrip_and_determinism() {
        for seed in 0..500u64 {
            let g = Genome::random(seed);
            assert_eq!(g, Genome::random(seed));
            assert_eq!(Genome::decode(&g.encode()).unwrap(), g);
            assert_eq!(Genome::from_hex(&g.to_hex()).unwrap(), g);
            assert!(
                g.validate(catalog()).is_empty(),
                "{seed}: {:?}",
                g.validate(catalog())
            );
        }
    }

    #[test]
    fn traits_apply_and_reject() {
        let j = serde_json::json!({
            "attributes": [
                {"trait_type": "top", "value": "dress"},
                {"trait_type": "hair_color", "value": "silver"},
                {"trait_type": "height_cm", "value": 170},
                {"trait_type": "face", "value": "nose-wide:0.5"}
            ],
            "phenotypes": ["idosa", "forte"]
        });
        let g = from_traits(&j, 3).unwrap();
        let cat = catalog();
        assert_eq!(cat.tops[g.top as usize].id, "dress");
        assert_eq!(g.bottom, 0);
        assert!(g.phenotype(genome::pheno::AGE) > 0.9);
        let h = genome::body_height_m(
            g.phenotype(genome::pheno::GENDER),
            g.phenotype(genome::pheno::AGE),
            g.phenotype(genome::pheno::HEIGHT),
        );
        assert!((h - 1.70).abs() < 0.02, "{h}");
        let bad = serde_json::json!({"attributes": [{"trait_type": "top", "value": "spacesuit"}]});
        assert!(from_traits(&bad, 1).is_err());
        let bad = serde_json::json!({"phenotypes": ["gigante"]});
        assert!(from_traits(&bad, 1).is_err());
    }

    #[test]
    fn residents_are_deterministic() {
        let a = npc_identities("vila", 40, 9);
        assert_eq!(a, npc_identities("vila", 40, 9));
        assert_ne!(a, npc_identities("vila", 40, 10));
        for r in &a {
            assert!(r.genome.validate(catalog()).is_empty());
        }
        let s = identity_prompt_schema();
        assert!(
            s["properties"]["appearance"]["properties"]["attributes"]["items"]["oneOf"]
                .as_array()
                .unwrap()
                .len()
                > 20
        );
    }
}
