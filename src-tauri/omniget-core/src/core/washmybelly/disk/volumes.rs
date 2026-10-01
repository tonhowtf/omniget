//! Mounted volumes the person would recognise, with total, free and, on
//! macOS, the purgeable space the system frees on its own when it needs it.

use std::path::Path;

use serde_json::{json, Value};

use super::super::{current_os, Os};

/// Filesystems that are never a place for files: kernel views, overlays,
/// automounters, snap images.
const PSEUDO_FS: &[&str] = &[
    "devfs",
    "autofs",
    "nullfs",
    "proc",
    "sysfs",
    "devtmpfs",
    "tmpfs",
    "devpts",
    "cgroup",
    "cgroup2",
    "overlay",
    "squashfs",
    "mqueue",
    "debugfs",
    "tracefs",
    "securityfs",
    "pstore",
    "bpf",
    "configfs",
    "fusectl",
    "hugetlbfs",
    "efivarfs",
    "binfmt_misc",
    "nsfs",
    "ramfs",
    "rpc_pipefs",
    "fuse.portal",
    "fuse.gvfsd-fuse",
    "fuse.snapfuse",
    "fuse.lxcfs",
    "selinuxfs",
    "tracefs",
    "nfsd",
    "zfs_snapshot",
];

/// Whether a mount stays out of the list. Pure, so every OS tests it.
pub fn hidden(os: Os, mount: &str, fs: &str) -> bool {
    let fs = fs.to_ascii_lowercase();
    if PSEUDO_FS.contains(&fs.as_str()) {
        return true;
    }
    match os {
        Os::Macos => {
            (mount.starts_with("/System/Volumes/") && mount != "/System/Volumes/Data")
                || mount.starts_with("/Library/Developer/CoreSimulator/")
                || mount.starts_with("/private/var/")
                || mount == "/dev"
                || mount.contains("/.timemachine/")
                || mount.starts_with("/Volumes/com.apple.TimeMachine.")
        }
        Os::Linux => {
            let under = |p: &str| mount == p || mount.starts_with(&format!("{p}/"));
            (under("/run") && !mount.starts_with("/run/media/"))
                || under("/proc")
                || under("/sys")
                || under("/dev")
                || under("/snap")
                || under("/var/lib/snapd")
                || under("/var/lib/docker")
                || mount == "/boot/efi"
                || mount == "/efi"
        }
        Os::Windows => false,
    }
}

/// Container size and free space from `diskutil info -plist` (already
/// parsed), plus whether the volume is a read-only system snapshot.
pub fn mac_info(info: &Value) -> Value {
    json!({
        "volume_name": info["VolumeName"].as_str().filter(|s| !s.is_empty()),
        "container_size": info["APFSContainerSize"].as_u64(),
        "container_free": info["APFSContainerFree"].as_u64(),
        "snapshot": info["APFSSnapshot"].as_bool().unwrap_or(false),
        "internal": info["Internal"].as_bool(),
        "solid_state": info["SolidState"].as_bool(),
        "device": info["DeviceIdentifier"].as_str(),
    })
}

/// Space a regular process may write right now (statvfs `f_bavail`).
#[cfg(unix)]
fn statvfs_free(mount: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(mount.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid C string and `s` a zeroed out-param.
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    Some((s.f_bavail as u64).saturating_mul(s.f_frsize as u64))
}

#[cfg(target_os = "macos")]
fn diskutil_info(target: &str) -> Option<Value> {
    let out = crate::core::process::std_command("diskutil")
        .args(["info", "-plist", target])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    super::plist::parse(&String::from_utf8_lossy(&out.stdout)).ok()
}

pub fn list() -> Vec<Value> {
    use sysinfo::{DiskKind, Disks};
    let os = current_os();
    let home = dirs::home_dir();
    let disks = Disks::new_with_refreshed_list();
    let mut out: Vec<Value> = Vec::new();
    let mut seen: Vec<(String, u64, u64)> = Vec::new();
    #[cfg(target_os = "macos")]
    let root_name = diskutil_info("/").and_then(|v| v["VolumeName"].as_str().map(str::to_string));
    let has_data = disks
        .list()
        .iter()
        .any(|d| d.mount_point() == Path::new("/System/Volumes/Data"));

    let mut list: Vec<_> = disks.list().iter().collect();
    // Shortest mount first, so a btrfs subvolume dedupes onto `/`.
    list.sort_by_key(|d| d.mount_point().as_os_str().len());
    for d in list {
        let mount = d.mount_point().to_string_lossy().into_owned();
        let fs = d.file_system().to_string_lossy().into_owned();
        if hidden(os, &mount, &fs) || d.total_space() == 0 {
            continue;
        }
        // On macOS `/` is the sealed system snapshot; the Data volume next
        // to it holds everything the person owns.
        if os == Os::Macos && mount == "/" && has_data {
            continue;
        }
        let device = d.name().to_string_lossy().into_owned();
        let key = (device.clone(), d.total_space(), d.available_space());
        if os == Os::Linux && !device.is_empty() && seen.contains(&key) {
            continue;
        }
        seen.push(key);

        let total = d.total_space();
        // sysinfo gives the "important usage" figure on macOS, which counts
        // purgeable space as available.
        let available = d.available_space();
        #[cfg(unix)]
        let free = statvfs_free(d.mount_point())
            .unwrap_or(available)
            .min(total);
        #[cfg(not(unix))]
        let free = available.min(total);
        let purgeable = (os == Os::Macos).then(|| available.saturating_sub(free));
        #[allow(unused_mut)]
        let mut name = device.clone();
        #[allow(unused_mut)]
        let mut extra = Value::Null;
        #[cfg(target_os = "macos")]
        {
            if let Some(info) = diskutil_info(&mount) {
                let m = mac_info(&info);
                if m["snapshot"] == true && mount != "/" {
                    continue;
                }
                if let Some(n) = m["volume_name"].as_str() {
                    name = n.to_string();
                }
                extra = m;
            }
            if mount == "/System/Volumes/Data" {
                name = root_name.clone().unwrap_or_else(|| "Macintosh HD".into());
            }
        }
        if name.is_empty() {
            name = mount.clone();
        }
        let kind = match d.kind() {
            DiskKind::SSD => "ssd",
            DiskKind::HDD => "hdd",
            _ => "unknown",
        };
        let holds_home = home.as_deref().is_some_and(|h| {
            h.starts_with(&mount)
                || (os == Os::Macos && mount == "/System/Volumes/Data" && h.starts_with("/Users"))
        });
        out.push(json!({
            "name": name,
            "mount": mount,
            "device": device,
            "file_system": fs,
            "total": total,
            "free": free,
            "available": available.max(free),
            "used": total.saturating_sub(free),
            "purgeable": purgeable,
            "removable": d.is_removable(),
            "read_only": d.is_read_only(),
            "kind": kind,
            "holds_home": holds_home,
            "info": extra,
        }));
    }
    // The volume that holds home comes first, removable ones last.
    out.sort_by_key(|v| (v["holds_home"] != true, v["removable"] == true));
    out
}
