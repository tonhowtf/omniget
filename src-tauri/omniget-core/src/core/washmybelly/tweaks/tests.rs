//! End-to-end tests of the tweak engine on the pretend Windows in
//! [`super::fake`]: state detection, snapshot and undo, the helper's form,
//! rollback on failure, power plans, and the helper's allow list.

use serde_json::{json, Value};

use super::super::elevate::{ElevatedOp, OpResult};
use super::super::plan::Source;
use super::super::store::Ctx;
use super::apply;
use super::fake::{FakeSys, SID};
use super::state::{self, Facts};
use super::*;

const ADV: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";

fn facts() -> Facts {
    Facts {
        build: 26100,
        edition: "Professional".into(),
        hags_blocked: None,
        ultimate_guid: None,
    }
}

fn item<'a>(list: &'a Value, id: &str) -> &'a Value {
    list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == id)
        .unwrap_or_else(|| panic!("{id} not listed"))
}

/// Runs helper operations on the fake machine through the real executors.
fn helper(sys: &FakeSys) -> impl Fn(&[ElevatedOp]) -> Result<Vec<OpResult>, String> + '_ {
    move |ops: &[ElevatedOp]| {
        Ok(ops
            .iter()
            .map(|op| {
                let r = match op {
                    ElevatedOp::RegSet {
                        key,
                        name,
                        kind,
                        data,
                    } => apply::helper_reg_set(sys, key, name, kind, data),
                    ElevatedOp::RegDelete { key, name } => {
                        apply::helper_reg_delete(sys, key, name.as_deref(), key_sets().unwrap())
                    }
                    ElevatedOp::ServiceStartup { name, mode } => sys
                        .run_ok(&super::super::startup::service_argv(name, mode, true).unwrap())
                        .map(|_| "ok".into()),
                    // The daily restore point comes first in a real batch.
                    ElevatedOp::Maintenance { id } if id == super::RESTORE_POINT_ID => {
                        Ok("restore point".into())
                    }
                    other => Err(format!("fake helper: {other:?}")),
                };
                match r {
                    Ok(detail) => OpResult {
                        ok: true,
                        detail,
                        bytes: 0,
                    },
                    Err(detail) => OpResult {
                        ok: false,
                        detail,
                        bytes: 0,
                    },
                }
            })
            .collect())
    }
}

#[test]
fn the_list_reads_state_from_the_registry() {
    let sys = FakeSys::new();
    // Game Mode: absent means on (winutil DefaultState).
    sys.set(ADV, "DisallowShaking", "REG_DWORD", "1");
    sys.set(ADV, "HideFileExt", "REG_DWORD", "1");
    sys.set(
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Search",
        "SearchboxTaskbarMode",
        "REG_DWORD",
        "1",
    );
    let list = list_with(&sys, &facts(), None).unwrap();
    assert_eq!(item(&list, "sophia.aero_shake_off")["state"], "applied");
    assert_eq!(item(&list, "w11d.ShowKnownFileExt")["state"], "not_applied");
    assert_eq!(
        item(&list, "winutil.game_mode")["state"],
        "applied",
        "absent values mean Game Mode is on"
    );
    assert_eq!(
        item(&list, "sophia.jpeg_wallpaper_quality")["state"],
        "not_applied"
    );
    // The search style group resolves to the member that is in place.
    let g = list["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["id"] == "w11d.SearchIcon")
        .unwrap();
    assert_eq!(g["active"], "w11d.ShowSearchIconTb");
    // Win10-only items are left out on build 26100.
    assert!(list["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i["id"] != "w11d.Hide3dObjects"));
    assert!(list["plan_id"].is_null());
}

#[test]
fn unreadable_and_unsuitable_items_are_disabled_with_a_reason() {
    let sys = FakeSys::new();
    sys.0
        .lock()
        .unwrap()
        .denied
        .insert(r"HKEY_CURRENT_USER\CONTROL PANEL\DESKTOP".into());
    let mut f = facts();
    f.edition = "Core".into();
    f.hags_blocked = Some("no_dedicated_gpu");
    let list = list_with(&sys, &f, None).unwrap();
    let jpeg = item(&list, "sophia.jpeg_wallpaper_quality");
    assert_eq!(
        (
            jpeg["state"].as_str(),
            jpeg["available"].as_bool(),
            jpeg["reason"].as_str()
        ),
        (Some("unknown"), Some(false), Some("unreadable"))
    );
    assert_eq!(item(&list, "ours.updates_defer")["reason"], "edition");
    assert_eq!(item(&list, "sophia.hags")["reason"], "no_dedicated_gpu");
    assert_eq!(
        item(&list, "w11d.EnableWindowsSandbox")["reason"],
        "needs_helper_update"
    );
    assert_eq!(item(&list, "ours.hvci_off")["risk"], "extreme");
}

#[test]
fn a_user_tweak_snapshots_the_real_old_value_and_undo_restores_it() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = Ctx::at(dir.path());
    let sys = FakeSys::new();
    // The old value has an odd type on purpose: undo must bring the type back.
    sys.set(ADV, "DisallowShaking", "REG_SZ", "weird");
    let out = execute_with(
        &sys,
        &ctx,
        "rcpt_1",
        "apply",
        &json!({"id": "sophia.aero_shake_off"}),
    )
    .unwrap();
    assert_eq!(
        sys.get(ADV, "DisallowShaking"),
        Some(("REG_DWORD".into(), "1".into()))
    );
    let Some(Undo::Module { action, args, .. }) = out.undo else {
        panic!("no undo")
    };
    undo_with(&sys, &ctx, &action, &args, &|_| {
        panic!("no helper for a user tweak")
    })
    .unwrap();
    assert_eq!(
        sys.get(ADV, "DisallowShaking"),
        Some(("REG_SZ".into(), "weird".into()))
    );
    // A value that did not exist is removed again.
    let out = execute_with(
        &sys,
        &ctx,
        "rcpt_2",
        "apply",
        &json!({"id": "sophia.jpeg_wallpaper_quality"}),
    )
    .unwrap();
    assert!(sys
        .get(r"HKCU\Control Panel\Desktop", "JPEGImportQuality")
        .is_some());
    let Some(Undo::Module { action, args, .. }) = out.undo else {
        panic!()
    };
    undo_with(&sys, &ctx, &action, &args, &|_| unreachable!()).unwrap();
    assert!(sys
        .get(r"HKCU\Control Panel\Desktop", "JPEGImportQuality")
        .is_none());
    // Every write went through reg.exe argv, 64-bit view, never a shell.
    assert!(sys
        .calls()
        .iter()
        .all(|c| c[0] != "cmd" && c[0] != "powershell"));
    assert!(sys
        .calls()
        .iter()
        .filter(|c| c[0] == "reg" && c[1] == "add")
        .all(|c| c.last().unwrap() == "/reg:64"));
}

#[test]
fn a_key_the_tweak_created_goes_away_on_undo() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = Ctx::at(dir.path());
    let sys = FakeSys::new();
    sys.set(r"HKCU\Software\Classes\CLSID", "", "REG_SZ", "");
    let out = execute_with(
        &sys,
        &ctx,
        "r",
        "apply",
        &json!({"id": "w11d.RevertContextMenu"}),
    )
    .unwrap();
    let k = r"HKCU\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32";
    assert_eq!(sys.get(k, ""), Some(("REG_SZ".into(), String::new())));
    let list = list_with(&sys, &facts(), Some("system")).unwrap();
    assert_eq!(item(&list, "w11d.RevertContextMenu")["state"], "applied");
    let Some(Undo::Module { action, args, .. }) = out.undo else {
        panic!()
    };
    undo_with(&sys, &ctx, &action, &args, &|_| unreachable!()).unwrap();
    assert!(
        !sys.has_key(r"HKCU\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}"),
        "the created key is gone, not just its value"
    );
    assert!(
        sys.has_key(r"HKCU\Software\Classes\CLSID"),
        "the shared parent stays"
    );
}

#[test]
fn a_failed_step_puts_the_earlier_ones_back() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = Ctx::at(dir.path());
    let sys = FakeSys::new();
    sys.set(
        r"HKCU\Software\Microsoft\GameBar",
        "AllowAutoGameMode",
        "REG_DWORD",
        "0",
    );
    sys.0.lock().unwrap().fail_on = Some("AutoGameModeEnabled".into());
    let e = execute_with(
        &sys,
        &ctx,
        "r",
        "apply",
        &json!({"id": "winutil.game_mode"}),
    )
    .unwrap_err();
    assert!(e.contains("put back"), "{e}");
    assert_eq!(
        sys.get(r"HKCU\Software\Microsoft\GameBar", "AllowAutoGameMode"),
        Some(("REG_DWORD".into(), "0".into()))
    );
}

#[test]
fn an_admin_tweak_goes_to_the_helper_with_the_users_sid_and_undoes_there() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = Ctx::at(dir.path());
    let sys = FakeSys::new();
    sys.set(
        r"HKCU\System\GameConfigStore",
        "GameDVR_Enabled",
        "REG_DWORD",
        "1",
    );
    let (ops, undo) =
        elevated_with(&sys, &ctx, "r", "apply", &json!({"id": "w11d.DisableDVR"})).unwrap();
    // HKCU becomes HKU\<sid>; HKLM stays; nothing changed yet.
    assert!(ops.iter().any(|o| matches!(o, ElevatedOp::RegSet { key, .. } if key.starts_with(&format!("HKU\\{SID}\\System\\GameConfigStore")))));
    assert!(ops.iter().any(|o| matches!(o, ElevatedOp::RegSet { key, .. } if key.starts_with("HKLM\\SOFTWARE\\Policies\\Microsoft\\Windows\\GameDVR"))));
    assert_eq!(
        sys.get(r"HKCU\System\GameConfigStore", "GameDVR_Enabled"),
        Some(("REG_DWORD".into(), "1".into()))
    );
    assert!(helper(&sys)(&ops).unwrap().iter().all(|r| r.ok));
    assert_eq!(
        sys.get(r"HKCU\System\GameConfigStore", "GameDVR_Enabled"),
        Some(("REG_DWORD".into(), "0".into()))
    );
    assert_eq!(
        sys.get(
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\GameDVR",
            "AllowGameDVR"
        ),
        Some(("REG_DWORD".into(), "0".into()))
    );
    let Some(Undo::Module { action, args, .. }) = undo else {
        panic!()
    };
    let h = helper(&sys);
    undo_with(&sys, &ctx, &action, &args, &h).unwrap();
    assert_eq!(
        sys.get(r"HKCU\System\GameConfigStore", "GameDVR_Enabled"),
        Some(("REG_DWORD".into(), "1".into()))
    );
    assert!(sys
        .get(
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\GameDVR",
            "AllowGameDVR"
        )
        .is_none());
    assert!(
        !sys.has_key(r"HKLM\SOFTWARE\Policies\Microsoft\Windows\GameDVR"),
        "the policy key it created is removed"
    );
    assert!(
        sys.has_key(r"HKLM\SOFTWARE\Policies\Microsoft\Windows"),
        "the shared policy parent stays"
    );
}

#[test]
fn whole_keys_deleted_by_a_tweak_come_back_with_their_values() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = Ctx::at(dir.path());
    let sys = FakeSys::new();
    let k = r"HKLM\SOFTWARE\Classes\Directory\shellex\ContextMenuHandlers\Sharing";
    sys.set(k, "", "REG_SZ", "{f81e9010-6ea4-11ce-a7ff-00aa003ca9f6}");
    sys.set(&format!("{k}\\Sub"), "Extra", "REG_DWORD", "7");
    let (ops, undo) = elevated_with(
        &sys,
        &ctx,
        "r",
        "apply",
        &json!({"id": "w11d.HideGiveAccessTo"}),
    )
    .unwrap();
    assert!(ops
        .iter()
        .any(|o| matches!(o, ElevatedOp::RegDelete { name: None, .. })));
    let ok = helper(&sys)(&ops).unwrap();
    assert!(ok.iter().all(|r| r.ok), "{ok:?}");
    assert!(!sys.has_key(k));
    let Some(Undo::Module { action, args, .. }) = undo else {
        panic!()
    };
    let h = helper(&sys);
    undo_with(&sys, &ctx, &action, &args, &h).unwrap();
    assert_eq!(
        sys.get(k, ""),
        Some((
            "REG_SZ".into(),
            "{f81e9010-6ea4-11ce-a7ff-00aa003ca9f6}".into()
        ))
    );
    assert_eq!(
        sys.get(&format!("{k}\\Sub"), "Extra"),
        Some(("REG_DWORD".into(), "7".into()))
    );
}

#[test]
fn back_to_default_is_its_own_action_with_its_own_undo() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = Ctx::at(dir.path());
    let sys = FakeSys::new();
    sys.set(ADV, "DisallowShaking", "REG_DWORD", "1");
    let out = execute_with(
        &sys,
        &ctx,
        "r",
        "default",
        &json!({"id": "sophia.aero_shake_off"}),
    )
    .unwrap();
    assert_eq!(
        sys.get(ADV, "DisallowShaking"),
        Some(("REG_DWORD".into(), "0".into()))
    );
    let Some(Undo::Module { action, args, .. }) = out.undo else {
        panic!()
    };
    undo_with(&sys, &ctx, &action, &args, &|_| unreachable!()).unwrap();
    assert_eq!(
        sys.get(ADV, "DisallowShaking"),
        Some(("REG_DWORD".into(), "1".into()))
    );
    assert!(
        execute_with(
            &sys,
            &ctx,
            "r",
            "default",
            &json!({"id": "w11d.ShowSearchBoxTb"})
        )
        .is_err(),
        "no default to go back to"
    );
    assert!(
        execute_with(
            &sys,
            &ctx,
            "r",
            "rm -rf",
            &json!({"id": "sophia.aero_shake_off"})
        )
        .is_err(),
        "closed action list"
    );
}

#[test]
fn power_plans_go_back_to_the_plan_you_had() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = Ctx::at(dir.path());
    let sys = FakeSys::new();
    sys.0.lock().unwrap().active_scheme = state::HIGH_PERFORMANCE.into();
    let out = execute_with(
        &sys,
        &ctx,
        "r",
        "apply",
        &json!({"id": "ours.power_ultimate"}),
    )
    .unwrap();
    let created = "5b2cf1a0-1a2b-4c3d-8e9f-0a1b2c3d4e5f";
    assert_eq!(sys.0.lock().unwrap().active_scheme, created);
    assert_eq!(
        ctx.kv_get::<String>("tweaks.ultimate_guid").as_deref(),
        Some(created)
    );
    let mut f = facts();
    f.ultimate_guid = Some(created.into());
    let list = list_with(&sys, &f, Some("gaming")).unwrap();
    assert_eq!(item(&list, "ours.power_ultimate")["state"], "applied");
    let Some(Undo::Module { action, args, .. }) = out.undo else {
        panic!()
    };
    undo_with(&sys, &ctx, &action, &args, &|_| unreachable!()).unwrap();
    let s = sys.0.lock().unwrap();
    assert_eq!(
        s.active_scheme,
        state::HIGH_PERFORMANCE,
        "not Balanced: the plan that was active"
    );
    assert!(!s.schemes.contains(created), "the plan we added is removed");
}

#[test]
fn the_helper_refuses_what_is_off_limits() {
    let sys = FakeSys::new();
    let keys = key_sets().unwrap();
    for (k, n) in [
        (
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows Defender",
            "DisableAntiSpyware",
        ),
        (
            r"HKLM\SYSTEM\CurrentControlSet\Control\DeviceGuard",
            "EnableVirtualizationBasedSecurity",
        ),
        (r"HKLM\SYSTEM\CurrentControlSet\Services\UCPD", "Start"),
        (r"HKLM\SAM\SAM\Domains", "F"),
        (
            r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\SystemRestore",
            "SystemRestorePointCreationFrequency",
        ),
    ] {
        assert!(
            apply::helper_reg_set(&sys, k, n, "REG_DWORD", "0")
                .unwrap_err()
                .starts_with("PROTECTED"),
            "{k}"
        );
    }
    assert!(sys.calls().is_empty(), "nothing ran");
    // Whole keys: an app's key the uninstaller backed up is accepted...
    sys.set(r"HKLM\SOFTWARE\FooCorp\FooApp", "x", "REG_SZ", "1");
    assert!(apply::helper_reg_delete(&sys, r"HKLM\SOFTWARE\FooCorp", None, keys).is_ok());
    assert!(!sys.has_key(r"HKLM\SOFTWARE\FooCorp"));
    // ...a shared one is not.
    assert!(apply::helper_reg_delete(&sys, r"HKLM\SOFTWARE\Microsoft", None, keys).is_err());
    assert!(
        apply::helper_reg_delete(&sys, r"HKLM\SYSTEM\CurrentControlSet\Control", None, keys)
            .is_err()
    );
    // The key-only form of RegSet creates nothing outside the roots.
    assert!(apply::helper_reg_set(&sys, r"HKLM\SYSTEM\Setup\X", "", apply::KEY_KIND, "").is_err());
}

#[test]
fn memory_integrity_is_only_for_the_screen() {
    let sys = FakeSys::new();
    let dir = tempfile::tempdir().unwrap();
    let ctx = Ctx::at(dir.path());
    let _ = list_with(&sys, &facts(), Some("gaming")).unwrap();
    let e = plan::from_ids(
        &ctx,
        PLAN_KIND,
        &["tweaks:ours.hvci_off".into()],
        Source::Agent,
    )
    .unwrap_err();
    assert!(e.starts_with("EXTREME_ITEM"), "{e}");
    let p = plan::from_ids(
        &ctx,
        PLAN_KIND,
        &["tweaks:ours.hvci_off".into()],
        Source::Ui,
    )
    .unwrap();
    assert!(p.needs_admin);
}

#[test]
fn the_restore_point_script_is_fixed_and_keeps_the_windows_limit() {
    let argv = restore_point_argv();
    assert_eq!(
        &argv[..4],
        &["powershell", "-NoProfile", "-NonInteractive", "-Command"]
    );
    assert!(
        argv[4].contains("Checkpoint-Computer") && argv[4].contains("$env:OMNIGET_WMB_RP_DESC")
    );
    assert!(!argv[4].contains("SystemRestorePointCreationFrequency"));
    assert_eq!(argv.len(), 5);
}
