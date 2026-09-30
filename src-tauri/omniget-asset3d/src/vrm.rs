//! VRM `meta` rules (estudo 02 §meta). The whole `meta` is kept verbatim in
//! the normalized file; here it becomes four yes/no answers, each `false`
//! unless the meta clearly says otherwise.

use crate::types::{Permissions, VrmInfo};
use serde_json::{Map, Value};

pub fn info(root_extensions: &Map<String, Value>) -> Option<VrmInfo> {
    if let Some(meta) = root_extensions.get("VRMC_vrm").and_then(|v| v.get("meta")) {
        return Some(VrmInfo {
            version: "1.0".into(),
            meta: meta.clone(),
            permissions: vrm_permissions(meta),
            credit_required: meta.get("creditNotation").and_then(Value::as_str)
                != Some("unnecessary"),
        });
    }
    if let Some(meta) = root_extensions.get("VRM").and_then(|v| v.get("meta")) {
        let lic = meta
            .get("licenseName")
            .and_then(Value::as_str)
            .unwrap_or("");
        return Some(VrmInfo {
            version: "0.x".into(),
            meta: meta.clone(),
            permissions: vrm_permissions(meta),
            credit_required: lic != "CC0",
        });
    }
    None
}

/// Permissions from a VRM 1.0 or 0.x `meta` object. Missing or unknown
/// values give the most restrictive answer.
pub fn vrm_permissions(meta: &Value) -> Permissions {
    let s = |k: &str| meta.get(k).and_then(Value::as_str).unwrap_or("");
    if meta.get("avatarPermission").is_some()
        || meta.get("licenseUrl").is_some()
        || meta.get("modification").is_some()
    {
        // VRM 1.0
        let modification = s("modification");
        return Permissions {
            use_as_avatar: s("avatarPermission") == "everyone",
            redistribute_to_others: meta.get("allowRedistribution").and_then(Value::as_bool)
                == Some(true),
            modify: matches!(
                modification,
                "allowModification" | "allowModificationRedistribution"
            ),
            export: modification == "allowModificationRedistribution",
        };
    }
    // VRM 0.x
    let lic = s("licenseName");
    let redistribute = matches!(
        lic,
        "CC0" | "CC_BY" | "CC_BY_NC" | "CC_BY_SA" | "CC_BY_NC_SA" | "CC_BY_ND" | "CC_BY_NC_ND"
    );
    let modify = matches!(
        lic,
        "CC0" | "CC_BY" | "CC_BY_NC" | "CC_BY_SA" | "CC_BY_NC_SA"
    );
    Permissions {
        use_as_avatar: s("allowedUserName") == "Everyone",
        redistribute_to_others: redistribute,
        modify,
        export: modify && redistribute,
    }
}

/// Most restrictive merge, for an export built from several pieces.
pub fn merge(a: Permissions, b: Permissions) -> Permissions {
    Permissions {
        use_as_avatar: a.use_as_avatar && b.use_as_avatar,
        redistribute_to_others: a.redistribute_to_others && b.redistribute_to_others,
        modify: a.modify && b.modify,
        export: a.export && b.export,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_are_restrictive() {
        assert_eq!(vrm_permissions(&json!({})), Permissions::default());
        assert_eq!(
            vrm_permissions(&json!({"licenseUrl": "https://vrm.dev/licenses/1.0/"})),
            Permissions::default()
        );
    }

    #[test]
    fn open_vrm1() {
        let p = vrm_permissions(&json!({
            "avatarPermission": "everyone", "allowRedistribution": true,
            "modification": "allowModificationRedistribution"
        }));
        assert!(p.use_as_avatar && p.redistribute_to_others && p.modify && p.export);
    }

    #[test]
    fn vrm0_prohibited() {
        let p = vrm_permissions(
            &json!({"licenseName": "Redistribution_Prohibited", "allowedUserName": "OnlyAuthor"}),
        );
        assert_eq!(p, Permissions::default());
    }
}
