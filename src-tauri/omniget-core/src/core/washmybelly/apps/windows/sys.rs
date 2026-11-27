//! Inventory steps follow Bulk Crap Uninstaller (Copyright (c) 2017 Marcin
//! Szeniak, Apache License 2.0, see `data/washmybelly/bcu/NOTICE`), with

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::super::App;
use super::inventory::{self, DiskProbe, ListProbe};
use super::leftovers::{self, Found, Other, Target};
use super::parse::{self, Root};
use super::registry;
use super::run::{self, sys::Ended, Planned};
use crate::core::washmybelly::progress;

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

fn env_dir(k: &str) -> Option<PathBuf> {
    env(k).map(PathBuf::from)
}

pub fn registry_apps() -> Vec<App> {
    let mut apps = Vec::new();
    for root in Root::ALL {
        for e in registry::read_entries(root) {
            if let Some(a) = inventory::entry_to_app(&e, &ListProbe, &env) {
                inventory::merge_into(&mut apps, a);
            }
        }
    }
    apps
}

fn wide_buf(len: usize) -> Vec<u16> {
    vec![0u16; len]
}

fn msi_property(code: &[u16], context: i32, prop: windows_sys::core::PCWSTR) -> Option<String> {
    use windows_sys::Win32::System::ApplicationInstallationAndServicing::MsiGetProductInfoExW;
    let mut len: u32 = 256;
    for _ in 0..3 {
        let mut buf = wide_buf(len as usize + 1);
        let mut cap = len + 1;
        let rc = unsafe {
            MsiGetProductInfoExW(
                code.as_ptr(),
                std::ptr::null(),
                context,
                prop,
                buf.as_mut_ptr(),
                &mut cap,
            )
        };
        match rc {
            0 => {
                let s = String::from_utf16_lossy(&buf[..cap as usize]);
                return Some(s).filter(|s| !s.trim().is_empty());
            }
            234 => len = cap + 1,
            _ => return None,
        }
    }
    None
}

pub fn msi_products() -> Vec<inventory::MsiProduct> {
    use windows_sys::Win32::System::ApplicationInstallationAndServicing::{
        MsiEnumProductsExW, INSTALLPROPERTY_INSTALLEDPRODUCTNAME, INSTALLPROPERTY_INSTALLLOCATION,
        INSTALLPROPERTY_PUBLISHER, INSTALLPROPERTY_VERSIONSTRING, MSIINSTALLCONTEXT_ALL,
        MSIINSTALLCONTEXT_MACHINE,
    };
    let mut out = Vec::new();
    for i in 0u32..20_000 {
        let mut code = wide_buf(39);
        let mut ctx: i32 = 0;
        let rc = unsafe {
            MsiEnumProductsExW(
                std::ptr::null(),
                std::ptr::null(),
                MSIINSTALLCONTEXT_ALL as u32,
                i,
                code.as_mut_ptr(),
                &mut ctx,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if rc == 259 {
            break;
        }
        if rc != 0 {
            continue;
        }
        let code_s = String::from_utf16_lossy(&code[..38]);
        if !parse::is_guid(&code_s) {
            continue;
        }
        let Some(name) = msi_property(&code, ctx, INSTALLPROPERTY_INSTALLEDPRODUCTNAME) else {
            continue;
        };
        out.push(inventory::MsiProduct {
            code: code_s,
            name,
            version: msi_property(&code, ctx, INSTALLPROPERTY_VERSIONSTRING),
            publisher: msi_property(&code, ctx, INSTALLPROPERTY_PUBLISHER),
            location: msi_property(&code, ctx, INSTALLPROPERTY_INSTALLLOCATION),
            machine: ctx == MSIINSTALLCONTEXT_MACHINE,
        });
    }
    out
}

fn winget_program() -> Option<PathBuf> {
    crate::core::washmybelly::packages::find_program("winget")
}

pub fn winget_rows() -> Vec<parse::WingetRow> {
    let Some(exe) = winget_program() else {
        return Vec::new();
    };
    let mut cmd = crate::core::process::std_command(&exe);
    cmd.args([
        "list",
        "--accept-source-agreements",
        "--disable-interactivity",
    ]);
    match run::sys::output_with_timeout(cmd, 120) {
        Ok(o) => parse::parse_winget_list(&String::from_utf8_lossy(&o.stdout)),
        Err(_) => Vec::new(),
    }
}

const APPX_LIST_SCRIPT: &str = "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-AppxPackage | Select-Object Name,PackageFullName,PackageFamilyName,InstallLocation,SignatureKind,IsFramework,IsResourcePackage,NonRemovable,Version,Publisher | ConvertTo-Json -Depth 3 -Compress";

pub fn appx_packages() -> Vec<parse::AppxPkg> {
    let mut cmd = std::process::Command::new(run::powershell(&run::system_root()));
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        APPX_LIST_SCRIPT,
    ]);
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    match run::sys::output_with_timeout(cmd, 90) {
        Ok(o) => parse::parse_appx_json(&String::from_utf8_lossy(&o.stdout)),
        Err(_) => Vec::new(),
    }
}

pub fn steam_exe() -> Option<String> {
    registry::value(r"HKCU\Software\Valve\Steam", "SteamExe")
        .map(|s| s.replace('/', "\\"))
        .filter(|s| Path::new(s).is_file())
}

pub fn steam_libraries() -> Vec<String> {
    let Some(root) =
        registry::value(r"HKCU\Software\Valve\Steam", "SteamPath").map(|s| s.replace('/', "\\"))
    else {
        return Vec::new();
    };
    let vdf = Path::new(&root)
        .join("steamapps")
        .join("libraryfolders.vdf");
    let mut libs = std::fs::read_to_string(&vdf)
        .map(|t| parse::steam_libraries(&t))
        .unwrap_or_default();
    if libs.is_empty() {
        libs.push(root);
    }
    libs
}

fn scoop_roots() -> Vec<(PathBuf, bool)> {
    let mut v = Vec::new();
    if let Some(d) = env_dir("SCOOP").or_else(|| env_dir("USERPROFILE").map(|h| h.join("scoop"))) {
        v.push((d, false));
    }
    if let Some(d) =
        env_dir("SCOOP_GLOBAL").or_else(|| env_dir("ProgramData").map(|p| p.join("scoop")))
    {
        v.push((d, true));
    }
    v
}

const STEPS: u64 = 7;

fn base_apps() -> Vec<App> {
    progress(
        "apps_list",
        "registry",
        0,
        STEPS,
        "reading installed programs",
    );
    let mut apps = registry_apps();
    progress(
        "apps_list",
        "msi",
        1,
        STEPS,
        "reading Windows Installer products",
    );
    for p in msi_products() {
        inventory::merge_into(&mut apps, inventory::msi_to_app(&p));
    }
    apps
}

fn merge_local_apps(apps: &mut Vec<App>) {
    progress("apps_list", "steam", 3, STEPS, "reading Steam libraries");
    for a in inventory::steam_apps(&steam_libraries()) {
        inventory::merge_into(apps, a);
    }
    progress("apps_list", "scoop", 4, STEPS, "reading Scoop apps");
    for (root, global) in scoop_roots() {
        for a in inventory::scoop_apps(&root, global) {
            inventory::merge_into(apps, a);
        }
    }
}

pub fn list_quick() -> Vec<App> {
    let mut apps = base_apps();
    merge_local_apps(&mut apps);
    inventory::finalize(apps)
}

pub fn list() -> Result<Vec<App>, String> {
    let mut apps = base_apps();
    progress(
        "apps_list",
        "winget",
        2,
        STEPS,
        "asking winget and the Store",
    );
    let (rows, packages) = std::thread::scope(|s| {
        let winget = s.spawn(winget_rows);
        let packages = appx_packages();
        (winget.join().unwrap_or_default(), packages)
    });
    inventory::attach_winget(&mut apps, &rows);
    merge_local_apps(&mut apps);
    progress("apps_list", "appx", 5, STEPS, "reading Store apps");
    for p in packages {
        let a = inventory::appx_to_app(&p);
        let listed_by_catalog = a.extra["appx"]["classified_by"] != "signature";
        if p.signature != "System" || listed_by_catalog {
            apps.push(a);
        }
    }
    progress(
        "apps_list",
        "orphans",
        6,
        STEPS,
        "looking for program folders nobody claims",
    );
    let bases: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .filter_map(|k| env_dir(k))
        .collect();
    let mut bases_dedup: Vec<PathBuf> = Vec::new();
    for b in bases {
        if !bases_dedup.iter().any(|x| {
            parse::norm_path(&x.to_string_lossy()) == parse::norm_path(&b.to_string_lossy())
        }) {
            bases_dedup.push(b);
        }
    }
    let orphans = inventory::orphan_dirs(&bases_dedup, &apps);
    apps.extend(orphans);
    let apps = inventory::finalize(apps);
    progress(
        "apps_list",
        "done",
        STEPS,
        STEPS,
        format!("{} apps", apps.len()),
    );
    Ok(apps)
}

pub fn others_for(target_id: &str, target_name: &str) -> Vec<Other> {
    let tn = parse::normalize_name(target_name);
    registry_apps()
        .into_iter()
        .filter(|a| {
            a.id != target_id
                && parse::normalize_name(&a.name) != tn
                && a.extra.get("system_component") != Some(&json!(true))
        })
        .map(|a| Other {
            name: a.name,
            publisher: a.publisher,
            location: a.path,
        })
        .collect()
}

fn schtasks_rows() -> Vec<(String, String)> {
    let mut cmd =
        std::process::Command::new(format!(r"{}\System32\schtasks.exe", run::system_root()));
    cmd.args(["/query", "/fo", "csv", "/v", "/nh"]);
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    run::sys::output_with_timeout(cmd, 60)
        .map(|o| parse::schtasks_csv(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

pub fn leftovers(
    t: &Target,
    others: &[Other],
    uninstall_key: Option<(Root, String)>,
) -> Vec<Found> {
    let mut bases: Vec<(PathBuf, bool)> = Vec::new();
    for (k, admin) in [
        ("ProgramFiles", true),
        ("ProgramFiles(x86)", true),
        ("ProgramData", true),
        ("APPDATA", false),
        ("LOCALAPPDATA", false),
    ] {
        if let Some(d) = env_dir(k) {
            if !bases.iter().any(|(b, _)| b == &d) {
                bases.push((d, admin));
            }
        }
    }
    if let Some(l) = env_dir("LOCALAPPDATA") {
        bases.push((l.join("Programs"), false));
    }
    if let Some(h) = env_dir("USERPROFILE") {
        bases.push((h.join("AppData").join("LocalLow"), false));
    }
    let mut out = leftovers::scan_dirs(&bases, t, others);
    let keys = [
        (r"HKCU\Software", false),
        (r"HKLM\SOFTWARE", true),
        (r"HKLM\SOFTWARE\WOW6432Node", true),
    ];
    out.extend(leftovers::scan_keys(&registry::children, &keys, t, others));
    let mut menus = Vec::new();
    if let Some(a) = env_dir("APPDATA") {
        menus.push((a.join(r"Microsoft\Windows\Start Menu\Programs"), false));
    }
    if let Some(p) = env_dir("ProgramData") {
        menus.push((p.join(r"Microsoft\Windows\Start Menu\Programs"), true));
    }
    out.extend(leftovers::scan_shortcuts(&menus, t, others));
    out.extend(leftovers::scan_services(&registry::services(), t));
    out.extend(leftovers::scan_tasks(&schtasks_rows(), t));
    if let Some((root, key)) = uninstall_key {
        let full = format!(r"{}\{}", root.uninstall_key(), key);
        if registry::exists(&full) {
            let f = leftovers::Facts {
                uninstall_key: true,
                ..Default::default()
            };
            let (score, reasons) = leftovers::score(&f);
            out.push(Found {
                kind: "registry",
                path: full,
                bytes: 0,
                score,
                reasons,
                admin: root.machine(),
                service: None,
                base: None,
            });
        }
    }
    out
}

fn reg_exe() -> String {
    format!(r"{}\System32\reg.exe", run::system_root())
}

pub fn reg_export(key: &str, file: &Path) -> Result<(), String> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let out = std::process::Command::new(reg_exe())
        .arg("export")
        .arg(key)
        .arg(file)
        .args(["/y", "/reg:64"])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() && file.is_file() {
        Ok(())
    } else {
        Err(format!(
            "could not back up {key}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

pub fn reg_delete(key: &str) -> Result<(), String> {
    let out = std::process::Command::new(reg_exe())
        .arg("delete")
        .arg(key)
        .args(["/f", "/reg:64"])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "could not remove {key}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

pub fn reg_import(file: &Path) -> Result<(), String> {
    let out = std::process::Command::new(reg_exe())
        .arg("import")
        .arg(file)
        .arg("/reg:64")
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("could not restore the key: {} (keys under HKLM need an administrator: reg import \"{}\")", String::from_utf8_lossy(&out.stderr).trim(), file.display()))
    }
}

pub fn run_planned(p: &Planned, label: &str) -> Result<i32, String> {
    match run::sys::run(p, label)? {
        Ended::Exit(code) => Ok(code),
        Ended::Stalled if p.msi_like => Err(format!("{label}: the Windows Installer shows no activity; it was left running, check it before trying again")),
        Ended::Stalled => Err(format!("{label}: the uninstaller stopped responding and was closed")),
        Ended::TimedOut => Err(format!("{label}: the uninstaller took too long")),
    }
}

pub fn running_in(dir: &str) -> Vec<String> {
    if dir.trim().len() < 4 {
        return Vec::new();
    }
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing().with_exe(sysinfo::UpdateKind::OnlyIfNotSet),
    );
    let mut names: Vec<String> = sys
        .processes()
        .values()
        .filter(|p| {
            p.exe()
                .is_some_and(|e| parse::path_inside(&e.to_string_lossy(), dir))
        })
        .map(|p| p.name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names.dedup();
    names
}

pub fn uninstall(args: &Value) -> Result<(String, u64), String> {
    let app_id = run::arg(args, "app_id").ok_or("app_id missing")?;
    let label = run::arg(args, "name").unwrap_or(app_id).to_string();
    let method = run::arg(args, "kind").unwrap_or("");
    let path = run::arg(args, "path").map(str::to_string);
    if let Some(p) = &path {
        let open = running_in(p);
        if !open.is_empty() {
            return Err(format!(
                "close {} first ({} is still running)",
                label,
                open.join(", ")
            ));
        }
    }
    let before = path
        .as_deref()
        .map(|p| crate::core::washmybelly::fsops::measure(Path::new(p)).0)
        .unwrap_or(0);
    let detail = match method {
        "steam" => {
            let appid = run::arg(args, "steam_appid").ok_or("steam app id missing")?;
            let exe = steam_exe().ok_or("Steam was not found")?;
            let p = run::steam_uninstall(&exe, appid)?;
            run_planned(&p, &label)?;
            format!("{label}: Steam asks you to confirm the removal")
        }
        "scoop" => {
            let name = run::arg(args, "scoop_name").ok_or("scoop name missing")?;
            let global = args
                .get("scoop_global")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let installed = scoop_roots()
                .into_iter()
                .any(|(r, g)| g == global && r.join("apps").join(name).is_dir());
            if !installed {
                return Err(format!("NOT_INSTALLED: {label} is not installed anymore"));
            }
            let p = run::scoop_uninstall(name, global, &run::system_root())?;
            let code = run_planned(&p, &label)?;
            if code != 0 {
                return Err(format!("{label}: scoop failed with code {code}"));
            }
            format!("{label}: uninstalled")
        }
        "winget" => {
            let id = run::arg(args, "winget_id").ok_or("winget id missing")?;
            let p = run::winget_uninstall(id)?;
            let m = parse::winget_exit(run_planned(&p, &label)?);
            if !m.ok {
                return Err(run::summary(&m, &label));
            }
            run::summary(&m, &label)
        }
        _ => uninstall_registered(args, app_id, &label)?,
    };
    let after = path
        .as_deref()
        .map(|p| crate::core::washmybelly::fsops::measure(Path::new(p)).0)
        .unwrap_or(0);
    Ok((detail, before.saturating_sub(after)))
}

pub fn entry_of(args: &Value, app_id: &str) -> Result<parse::RegEntry, String> {
    let root = run::arg(args, "reg_root")
        .and_then(Root::parse)
        .ok_or("registry root missing")?;
    let key = run::arg(args, "reg_key").ok_or("registry key missing")?;
    let agrees = app_id == format!("reg:{}:{}", root.as_str(), key)
        || app_id
            .strip_prefix("msi:")
            .is_some_and(|g| g.eq_ignore_ascii_case(key));
    if !agrees {
        return Err("the item does not match its registry entry".into());
    }
    registry::read_entry(root, key)
        .ok_or_else(|| format!("NOT_INSTALLED: {app_id} is not installed anymore"))
}

fn uninstall_registered(args: &Value, app_id: &str, label: &str) -> Result<String, String> {
    let entry = entry_of(args, app_id)?;
    let still_there = || registry::read_entry(entry.root, &entry.key_name).is_some();
    if let Some(id) = run::arg(args, "winget_id").filter(|id| parse::winget_id_ok(id)) {
        if winget_program().is_some() {
            let p = run::winget_uninstall(id)?;
            let m = parse::winget_exit(run_planned(&p, label)?);
            if m.ok && !still_there() {
                return Ok(run::summary(&m, label));
            }
            if !m.skipped && !m.ok {
                return Err(run::summary(&m, label));
            }
        }
    }
    let p = run::for_entry(&entry, &DiskProbe, &run::system_root())?;
    let code = run_planned(&p, label)?;
    let m = parse::installer_exit(p.kind, code);
    if m.ok && still_there() {
        return Err(format!("{label}: the uninstaller finished but the app is still installed (it may have been cancelled)"));
    }
    if !m.ok {
        return Err(run::summary(&m, label));
    }
    Ok(run::summary(&m, label))
}

pub fn appx_remove(name: &str) -> Result<String, String> {
    let p = run::appx_remove(name, &run::system_root())?;
    match run_planned(&p, name)? {
        0 => Ok(format!("{name}: removed for this account")),
        3 => Err(format!("NOT_INSTALLED: {name} is not installed anymore")),
        4 => Err(format!("{name}: Windows kept the package")),
        c => Err(format!("{name}: PowerShell failed with code {c}")),
    }
}

pub fn store_reinstall(store_id: &str) -> Result<String, String> {
    let p = run::store_install(store_id)?;
    let m = parse::winget_exit(run_planned(&p, store_id)?);
    if m.ok || m.skipped {
        Ok(format!("reinstalled from the Microsoft Store ({store_id})"))
    } else {
        Err(m.text)
    }
}
