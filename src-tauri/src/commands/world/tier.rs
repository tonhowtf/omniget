//! Render tier: what the calibration measured, and what the user pinned.
//! Owned by f7-world-bridge.
//!
//! Plan §1.3 is explicit that no string in the webview reveals llvmpipe, so the
//! tier is a *measurement*, never a guess from a GPU name, a core count or an
//! OS. The route runs the calibration scene once and hands the result here;
//! this file keeps it in `<app_data>/world/profile.json` so the next launch
//! does not pay for it again, and keeps the user's override beside it.
//!
//! `pinned` wins over `measured`. A user who drags the slider down to tier 0
//! because the fan is loud must not have the watchdog or the next calibration
//! quietly undo it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::save::world_dir;

/// Highest tier the renderer knows (plan §1.2: 0 software … 3 high).
pub const MAX_TIER: u8 = 3;

/// `<root>/world/profile.json`.
pub fn profile_path(root: &Path) -> PathBuf {
    world_dir(root).join("profile.json")
}

/// What the world knows about this machine's rendering.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldProfile {
    /// Tier the user pinned in Settings. `None` = follow the measurement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<u8>,
    /// Tier the calibration scene arrived at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measured: Option<u8>,
    /// Median frame time of the calibration pass, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub median_ms: Option<f64>,
    /// `gl2` | `gl1` | `canvas2d` | `none`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    /// The calibration lost the GL context on the way, which is a tier 0 smell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_lost: Option<bool>,
    /// Unix milliseconds of the last calibration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibrated_at_ms: Option<u64>,
}

impl WorldProfile {
    /// The tier the renderer should actually use, or `None` when nothing is
    /// known yet and the route has to calibrate.
    pub fn effective(&self) -> Option<u8> {
        self.pinned.or(self.measured)
    }
}

/// Refuse a tier outside 0..=3 instead of clamping: a UI sending 9 has a bug,
/// and silently rendering tier 3 would hide it.
pub fn check_tier(tier: u8) -> Result<u8, String> {
    if tier > MAX_TIER {
        return Err(format!("ERR_WORLD_BAD_TIER: {tier}"));
    }
    Ok(tier)
}

/// Read the profile; a missing or corrupt file is an empty profile, never an
/// error, because losing the calibration costs one second on the next open.
pub fn load_profile(root: &Path) -> WorldProfile {
    std::fs::read_to_string(profile_path(root))
        .ok()
        .and_then(|s| serde_json::from_str::<WorldProfile>(&s).ok())
        .unwrap_or_default()
}

pub fn store_profile(root: &Path, p: &WorldProfile) -> Result<(), String> {
    let dir = world_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| format!("ERR_WORLD_PROFILE: {e}"))?;
    let body = serde_json::to_string_pretty(p).map_err(|e| format!("ERR_WORLD_PROFILE: {e}"))?;
    let tmp = dir.join("profile.json.tmp");
    std::fs::write(&tmp, body).map_err(|e| format!("ERR_WORLD_PROFILE: {e}"))?;
    std::fs::rename(&tmp, profile_path(root)).map_err(|e| format!("ERR_WORLD_PROFILE: {e}"))?;
    Ok(())
}

/// Fold a `CalibrationResult` from the renderer into an existing profile. The
/// pin is deliberately untouched.
pub fn apply_calibration(
    prev: &WorldProfile,
    result: &serde_json::Value,
    now_ms: u64,
) -> Result<WorldProfile, String> {
    let tier = result
        .get("tier")
        .and_then(|v| v.as_u64())
        .ok_or("ERR_WORLD_BAD_CALIBRATION: tier")?;
    if tier > MAX_TIER as u64 {
        return Err(format!("ERR_WORLD_BAD_TIER: {tier}"));
    }
    let median = result
        .get("medianMs")
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v >= 0.0);
    Ok(WorldProfile {
        pinned: prev.pinned,
        measured: Some(tier as u8),
        median_ms: median,
        backend: result
            .get("backend")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        context_lost: result.get("contextLostDuring").and_then(|v| v.as_bool()),
        calibrated_at_ms: Some(now_ms),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("omniget-world-tier-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_profile_reads_as_empty_and_knows_no_tier() {
        let root = tmp_root("missing");
        let p = load_profile(&root);
        assert_eq!(p, WorldProfile::default());
        assert_eq!(p.effective(), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_corrupt_profile_reads_as_empty_instead_of_failing() {
        let root = tmp_root("corrupt");
        std::fs::create_dir_all(world_dir(&root)).unwrap();
        std::fs::write(profile_path(&root), "{ not json").unwrap();
        assert_eq!(load_profile(&root), WorldProfile::default());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_profile_round_trips_through_the_disk() {
        let root = tmp_root("round-trip");
        let p = WorldProfile {
            pinned: Some(1),
            measured: Some(3),
            median_ms: Some(4.25),
            backend: Some("gl2".into()),
            context_lost: Some(false),
            calibrated_at_ms: Some(1_700_000_000_000),
        };
        store_profile(&root, &p).unwrap();
        assert_eq!(load_profile(&root), p);
        assert!(!world_dir(&root).join("profile.json.tmp").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_pin_wins_over_the_measurement() {
        let mut p = WorldProfile {
            measured: Some(3),
            ..Default::default()
        };
        assert_eq!(p.effective(), Some(3));
        p.pinned = Some(0);
        assert_eq!(p.effective(), Some(0));
        p.pinned = None;
        assert_eq!(p.effective(), Some(3));
    }

    #[test]
    fn a_tier_outside_the_range_is_refused() {
        assert_eq!(check_tier(0), Ok(0));
        assert_eq!(check_tier(MAX_TIER), Ok(MAX_TIER));
        assert!(check_tier(MAX_TIER + 1)
            .unwrap_err()
            .starts_with("ERR_WORLD_BAD_TIER"));
    }

    #[test]
    fn a_calibration_updates_the_measurement_and_leaves_the_pin_alone() {
        let prev = WorldProfile {
            pinned: Some(1),
            measured: Some(0),
            ..Default::default()
        };
        let next = apply_calibration(
            &prev,
            &serde_json::json!({
                "tier": 2,
                "medianMs": 7.5,
                "backend": "gl2",
                "contextLostDuring": false,
            }),
            42,
        )
        .unwrap();
        assert_eq!(next.pinned, Some(1), "the pin survives a recalibration");
        assert_eq!(next.measured, Some(2));
        assert_eq!(next.median_ms, Some(7.5));
        assert_eq!(next.backend.as_deref(), Some("gl2"));
        assert_eq!(next.calibrated_at_ms, Some(42));
        assert_eq!(next.effective(), Some(1));
    }

    #[test]
    fn a_calibration_without_a_tier_is_refused() {
        let err =
            apply_calibration(&WorldProfile::default(), &serde_json::json!({}), 0).unwrap_err();
        assert!(err.starts_with("ERR_WORLD_BAD_CALIBRATION"), "got {err}");
        let err = apply_calibration(
            &WorldProfile::default(),
            &serde_json::json!({ "tier": 9 }),
            0,
        )
        .unwrap_err();
        assert!(err.starts_with("ERR_WORLD_BAD_TIER"), "got {err}");
    }

    #[test]
    fn a_nonsense_median_is_dropped_rather_than_stored() {
        let next = apply_calibration(
            &WorldProfile::default(),
            &serde_json::json!({ "tier": 0, "medianMs": -1.0 }),
            0,
        )
        .unwrap();
        assert_eq!(next.median_ms, None);
    }
}
