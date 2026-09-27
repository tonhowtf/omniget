//! Physical disk health, only from what the OS hands out without root:
//! macOS `diskutil info -plist` (SMART status and the NVMe log on Apple
//! SSDs), Linux UDisks2 on the system bus (no `setcap` on `smartctl`),
//! Windows `Get-PhysicalDisk` and `Get-StorageReliabilityCounter`.
//!
//! Nothing is guessed: a value the OS did not give stays `null`, and when a
//! whole source is missing the answer says why.

use serde_json::{json, Map, Value};

/// `good`, `warning`, `failing` or `unknown`.
fn status(failing: bool, warning: bool, known: bool) -> &'static str {
    if failing {
        "failing"
    } else if warning {
        "warning"
    } else if known {
        "good"
    } else {
        "unknown"
    }
}

/// Kelvin from NVMe / UDisks to Celsius, rounded.
fn kelvin_to_c(k: f64) -> Option<i64> {
    (k > 200.0 && k < 500.0).then(|| (k - 273.15).round() as i64)
}

// ── macOS ────────────────────────────────────────────────────────────

/// The whole physical disks in `diskutil list -plist physical`.
pub fn mac_whole_disks(list: &Value) -> Vec<String> {
    list["WholeDisks"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|d| d.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// One disk from `diskutil info -plist diskN`.
pub fn mac_disk(info: &Value) -> Value {
    let smart = info["SMARTStatus"].as_str().unwrap_or("").to_string();
    let log = &info["SMARTDeviceSpecificKeysMayVaryNotGuaranteed"];
    let n = |k: &str| log[k].as_u64();
    let percent_used = n("PERCENTAGE_USED");
    let spare = n("AVAILABLE_SPARE");
    let spare_min = n("AVAILABLE_SPARE_THRESHOLD");
    let media_errors = n("MEDIA_ERRORS_0");
    let critical = n("CRITICAL_WARNING").unwrap_or(0);
    let failing = smart.eq_ignore_ascii_case("failing") || critical != 0;
    let mut warnings = Vec::new();
    if percent_used.is_some_and(|p| p >= 90) {
        warnings.push("wear");
    }
    if let (Some(s), Some(t)) = (spare, spare_min) {
        if s <= t {
            warnings.push("spare");
        }
    }
    if media_errors.is_some_and(|e| e > 0) {
        warnings.push("media_errors");
    }
    let known = smart.eq_ignore_ascii_case("verified");
    json!({
        "id": info["DeviceIdentifier"],
        "name": info["MediaName"].as_str().filter(|s| !s.is_empty()).or(info["IORegistryEntryName"].as_str()),
        "bus": info["BusProtocol"],
        "media": match info["SolidState"].as_bool() { Some(true) => json!("ssd"), Some(false) => json!("hdd"), None => Value::Null },
        "internal": info["Internal"],
        "removable": info["RemovableMediaOrExternalDevice"].as_bool().or(info["Removable"].as_bool()),
        "size": info["TotalSize"].as_u64().or(info["Size"].as_u64()),
        "status": status(failing, !warnings.is_empty(), known),
        "smart_status": if smart.is_empty() { Value::Null } else { json!(smart) },
        "warnings": warnings,
        "temperature_c": n("TEMPERATURE").and_then(|k| kelvin_to_c(k as f64)),
        "percent_used": percent_used,
        "available_spare": spare,
        "media_errors": media_errors,
        "power_on_hours": n("POWER_ON_HOURS_0"),
        "power_cycles": n("POWER_CYCLES_0"),
        "unsafe_shutdowns": n("UNSAFE_SHUTDOWNS_0"),
        // NVMe data units are 1000 blocks of 512 bytes.
        "bytes_written": n("DATA_UNITS_WRITTEN_0").map(|u| u.saturating_mul(512_000)),
        "bytes_read": n("DATA_UNITS_READ_0").map(|u| u.saturating_mul(512_000)),
        "source": "diskutil",
    })
}

#[cfg(target_os = "macos")]
fn diskutil(args: &[&str]) -> Result<Value, String> {
    let out = crate::core::process::std_command("diskutil")
        .args(args)
        .output()
        .map_err(|e| format!("diskutil: {e}"))?;
    if !out.status.success() {
        return Err(format!("diskutil {} failed", args.join(" ")));
    }
    super::plist::parse(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(target_os = "macos")]
fn read() -> Value {
    let list = match diskutil(&["list", "-plist", "physical"]) {
        Ok(l) => l,
        Err(e) => return json!({ "disks": [], "reason": e }),
    };
    let disks: Vec<Value> = mac_whole_disks(&list)
        .iter()
        // Disk ids come from diskutil itself (`disk0`), never from a caller.
        .filter(|d| {
            d.len() > 4 && d.starts_with("disk") && d[4..].chars().all(|c| c.is_ascii_digit())
        })
        .filter_map(|d| diskutil(&["info", "-plist", d]).ok())
        .map(|i| mac_disk(&i))
        .collect();
    let mut o = json!({ "disks": disks });
    if o["disks"].as_array().is_some_and(|a| a.is_empty()) {
        o["reason"] = json!("diskutil listed no physical disk");
    }
    o
}

// ── Linux (UDisks2) ──────────────────────────────────────────────────

const DRIVE: &str = "org.freedesktop.UDisks2.Drive";
const ATA: &str = "org.freedesktop.UDisks2.Drive.Ata";
const NVME: &str = "org.freedesktop.UDisks2.NVMe.Controller";

/// Disks from UDisks2 `GetManagedObjects`, as JSON
/// `{ object_path: { interface: { property: value } } }`.
pub fn udisks_disks(objects: &Value) -> Vec<Value> {
    let Some(map) = objects.as_object() else {
        return Vec::new();
    };
    let mut paths: Vec<&String> = map.keys().collect();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let ifaces = &map[path];
        let drive = &ifaces[DRIVE];
        if !drive.is_object() {
            continue;
        }
        let ata = &ifaces[ATA];
        let nvme = &ifaces[NVME];
        let model = [
            drive["Vendor"].as_str().unwrap_or(""),
            drive["Model"].as_str().unwrap_or(""),
        ]
        .iter()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
        let media = match drive["Rotational"].as_bool() {
            Some(true) => json!("hdd"),
            Some(false) => json!("ssd"),
            None => Value::Null,
        };
        let mut d = Map::new();
        d.insert("id".into(), json!(path.rsplit('/').next().unwrap_or(path)));
        d.insert("name".into(), json!(model));
        d.insert(
            "bus".into(),
            drive["ConnectionBus"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map_or(Value::Null, |s| json!(s)),
        );
        d.insert("media".into(), media);
        d.insert(
            "removable".into(),
            json!(
                drive["Removable"].as_bool().unwrap_or(false)
                    || drive["MediaRemovable"].as_bool().unwrap_or(false)
            ),
        );
        d.insert("size".into(), drive["Size"].clone());
        d.insert("source".into(), json!("udisks2"));
        let mut warnings: Vec<&str> = Vec::new();
        if ata.is_object()
            && ata["SmartSupported"].as_bool() == Some(true)
            && ata["SmartEnabled"].as_bool() != Some(false)
        {
            let failing = ata["SmartFailing"].as_bool() == Some(true);
            if ata["SmartNumBadSectors"].as_u64().is_some_and(|n| n > 0) {
                warnings.push("bad_sectors");
            }
            if ata["SmartNumAttributesFailing"]
                .as_i64()
                .is_some_and(|n| n > 0)
            {
                warnings.push("attributes_failing");
            }
            d.insert(
                "status".into(),
                json!(status(failing, !warnings.is_empty(), true)),
            );
            d.insert(
                "smart_status".into(),
                json!(if failing { "failing" } else { "passed" }),
            );
            d.insert(
                "temperature_c".into(),
                json!(ata["SmartTemperature"].as_f64().and_then(kelvin_to_c)),
            );
            d.insert(
                "power_on_hours".into(),
                json!(ata["SmartPowerOnSeconds"].as_u64().map(|s| s / 3600)),
            );
            d.insert("bad_sectors".into(), ata["SmartNumBadSectors"].clone());
        } else if nvme.is_object() && nvme["SmartUpdated"].as_u64().is_some_and(|t| t > 0) {
            let crit: Vec<String> = nvme["SmartCriticalWarning"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            // `spare` and `temperature` warn; the others mean the drive is
            // degraded or read-only.
            let failing = crit
                .iter()
                .any(|c| !matches!(c.as_str(), "spare" | "temperature"));
            if !crit.is_empty() && !failing {
                warnings.push("critical_warning");
            }
            d.insert(
                "status".into(),
                json!(status(failing, !warnings.is_empty(), true)),
            );
            d.insert(
                "smart_status".into(),
                json!(if crit.is_empty() {
                    "passed".to_string()
                } else {
                    crit.join(",")
                }),
            );
            d.insert(
                "temperature_c".into(),
                json!(nvme["SmartTemperature"].as_f64().and_then(kelvin_to_c)),
            );
            d.insert("power_on_hours".into(), nvme["SmartPowerOnHours"].clone());
        } else {
            d.insert("status".into(), json!("unknown"));
            d.insert("smart_status".into(), Value::Null);
            d.insert("temperature_c".into(), Value::Null);
            d.insert("power_on_hours".into(), Value::Null);
            d.insert(
                "reason".into(),
                json!("no SMART data from UDisks2 for this drive"),
            );
        }
        d.insert("warnings".into(), json!(warnings));
        out.push(Value::Object(d));
    }
    out
}

#[cfg(target_os = "linux")]
fn udisks_objects() -> Result<Value, String> {
    use std::collections::HashMap;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value as ZValue};

    fn conv(v: &ZValue) -> Value {
        match v {
            ZValue::Bool(b) => json!(b),
            ZValue::U8(n) => json!(n),
            ZValue::I16(n) => json!(n),
            ZValue::U16(n) => json!(n),
            ZValue::I32(n) => json!(n),
            ZValue::U32(n) => json!(n),
            ZValue::I64(n) => json!(n),
            ZValue::U64(n) => json!(n),
            ZValue::F64(n) => json!(n),
            ZValue::Str(s) => json!(s.as_str()),
            ZValue::ObjectPath(p) => json!(p.as_str()),
            ZValue::Value(b) => conv(b),
            ZValue::Array(a) => Value::Array(a.inner().iter().map(conv).collect()),
            _ => Value::Null,
        }
    }

    let conn = zbus::blocking::Connection::system().map_err(|e| format!("system bus: {e}"))?;
    let reply = conn
        .call_method(
            Some("org.freedesktop.UDisks2"),
            "/org/freedesktop/UDisks2",
            Some("org.freedesktop.DBus.ObjectManager"),
            "GetManagedObjects",
            &(),
        )
        .map_err(|e| format!("UDisks2: {e}"))?;
    let objs: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>> = reply
        .body()
        .deserialize()
        .map_err(|e| format!("UDisks2 reply: {e}"))?;
    let mut out = Map::new();
    for (path, ifaces) in objs {
        let mut im = Map::new();
        for (iface, props) in ifaces {
            if iface != DRIVE && iface != ATA && iface != NVME {
                continue;
            }
            let pm: Map<String, Value> = props.iter().map(|(k, v)| (k.clone(), conv(v))).collect();
            im.insert(iface, Value::Object(pm));
        }
        if !im.is_empty() {
            out.insert(path.as_str().to_string(), Value::Object(im));
        }
    }
    Ok(Value::Object(out))
}

#[cfg(target_os = "linux")]
fn read() -> Value {
    match udisks_objects() {
        Ok(o) => {
            let disks = udisks_disks(&o);
            if disks.is_empty() {
                json!({ "disks": [], "reason": "UDisks2 reported no drives" })
            } else {
                json!({ "disks": disks })
            }
        }
        Err(e) => {
            json!({ "disks": [], "reason": format!("UDisks2 is not reachable ({e}); install or start udisks2 to see disk health") })
        }
    }
}

// ── Windows ──────────────────────────────────────────────────────────

/// Fixed script: no path or caller value is ever put into it.
#[cfg_attr(not(windows), allow(dead_code))]
pub const WINDOWS_SCRIPT: &str = "$ErrorActionPreference='SilentlyContinue'; \
Get-PhysicalDisk | ForEach-Object { $r = $_ | Get-StorageReliabilityCounter; \
[pscustomobject]@{ DeviceId=[string]$_.DeviceId; FriendlyName=$_.FriendlyName; MediaType=[string]$_.MediaType; \
BusType=[string]$_.BusType; HealthStatus=[string]$_.HealthStatus; OperationalStatus=[string]($_.OperationalStatus -join ','); \
Size=$_.Size; Temperature=$r.Temperature; TemperatureMax=$r.TemperatureMax; Wear=$r.Wear; PowerOnHours=$r.PowerOnHours; \
ReadErrorsTotal=$r.ReadErrorsTotal; WriteErrorsTotal=$r.WriteErrorsTotal; StartStopCycleCount=$r.StartStopCycleCount } } \
| ConvertTo-Json -Depth 3";

/// Disks from the script's JSON (one object or an array).
pub fn windows_disks(text: &str) -> Result<Vec<Value>, String> {
    let t = text.trim().trim_start_matches('\u{feff}');
    if t.is_empty() {
        return Ok(Vec::new());
    }
    let v: Value = serde_json::from_str(t).map_err(|e| format!("Get-PhysicalDisk: {e}"))?;
    let arr = match v {
        Value::Array(a) => a,
        o @ Value::Object(_) => vec![o],
        _ => return Err("Get-PhysicalDisk: unexpected output".into()),
    };
    Ok(arr
        .iter()
        .map(|d| {
            let health = d["HealthStatus"].as_str().unwrap_or("");
            let wear = d["Wear"].as_u64();
            let errors = d["ReadErrorsTotal"].as_u64().unwrap_or(0) + d["WriteErrorsTotal"].as_u64().unwrap_or(0);
            let mut warnings = Vec::new();
            if wear.is_some_and(|w| w >= 90) {
                warnings.push("wear");
            }
            if errors > 0 {
                warnings.push("io_errors");
            }
            if health.eq_ignore_ascii_case("warning") {
                warnings.push("health_warning");
            }
            let failing = health.eq_ignore_ascii_case("unhealthy");
            let known = !health.is_empty() && !health.eq_ignore_ascii_case("unknown");
            let media = match d["MediaType"].as_str().unwrap_or("") {
                "SSD" | "SCM" => json!("ssd"),
                "HDD" => json!("hdd"),
                _ => Value::Null,
            };
            json!({
                "id": d["DeviceId"],
                "name": d["FriendlyName"],
                "bus": d["BusType"],
                "media": media,
                "removable": d["BusType"].as_str().is_some_and(|b| b.eq_ignore_ascii_case("USB") || b.eq_ignore_ascii_case("SD")),
                "size": d["Size"],
                "status": status(failing, !warnings.is_empty(), known),
                "smart_status": if health.is_empty() { Value::Null } else { json!(health) },
                "operational_status": d["OperationalStatus"],
                "warnings": warnings,
                "temperature_c": d["Temperature"].as_i64().filter(|t| *t > 0),
                "percent_used": wear,
                "power_on_hours": d["PowerOnHours"],
                "read_errors": d["ReadErrorsTotal"],
                "write_errors": d["WriteErrorsTotal"],
                "source": "storage",
            })
        })
        .collect())
}

#[cfg(windows)]
fn read() -> Value {
    let out = crate::core::process::std_command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT])
        .output();
    match out {
        Ok(o) if o.status.success() => match windows_disks(&String::from_utf8_lossy(&o.stdout)) {
            Ok(d) if d.is_empty() => {
                json!({ "disks": [], "reason": "Get-PhysicalDisk listed no disk" })
            }
            Ok(d) => json!({ "disks": d }),
            Err(e) => json!({ "disks": [], "reason": e }),
        },
        Ok(o) => {
            json!({ "disks": [], "reason": format!("Get-PhysicalDisk failed: {}", String::from_utf8_lossy(&o.stderr).trim()) })
        }
        Err(e) => json!({ "disks": [], "reason": format!("powershell: {e}") }),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn read() -> Value {
    json!({ "disks": [], "reason": "disk health is not read on this system" })
}

pub fn health() -> Value {
    read()
}

#[cfg(test)]
mod tests {
    use super::super::plist;
    use super::*;

    fn fx(name: &str) -> String {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/washmybelly_fixtures/disk");
        std::fs::read_to_string(dir.join(name)).unwrap()
    }

    #[test]
    fn mac_nvme_from_diskutil() {
        let list = plist::parse(&fx("list_physical.plist")).unwrap();
        assert_eq!(mac_whole_disks(&list), ["disk0"]);
        let d = mac_disk(&plist::parse(&fx("disk0.plist")).unwrap());
        assert_eq!(d["id"], "disk0");
        assert_eq!(d["name"], "APPLE SSD AP1024Z");
        assert_eq!(d["media"], "ssd");
        assert_eq!(d["status"], "good");
        assert_eq!(d["smart_status"], "Verified");
        assert_eq!(d["temperature_c"], 50, "323 K in the fixture");
        assert_eq!(d["percent_used"], 1);
        assert!(d["power_on_hours"].as_u64().unwrap() > 0);
        assert!(d["bytes_written"].as_u64().unwrap() > 0);
    }

    #[test]
    fn mac_unsupported_is_unknown() {
        let d = mac_disk(
            &json!({ "DeviceIdentifier": "disk4", "MediaName": "USB", "SMARTStatus": "Not Supported", "SolidState": false }),
        );
        assert_eq!(d["status"], "unknown");
        assert_eq!(d["temperature_c"], Value::Null);
        let d = mac_disk(&json!({ "DeviceIdentifier": "disk5", "SMARTStatus": "Failing" }));
        assert_eq!(d["status"], "failing");
    }

    #[test]
    fn udisks_ata_nvme_and_usb() {
        let objs: Value = serde_json::from_str(&fx("udisks_objects.json")).unwrap();
        let d = udisks_disks(&objs);
        assert_eq!(d.len(), 3, "block devices are not drives");
        let nvme = d
            .iter()
            .find(|x| x["name"] == "Samsung SSD 970 EVO Plus 1TB")
            .unwrap();
        assert_eq!(nvme["status"], "good");
        assert_eq!(nvme["temperature_c"], 38);
        assert_eq!(nvme["media"], "ssd");
        let hdd = d
            .iter()
            .find(|x| x["name"] == "ATA ST2000DM008-2FR102")
            .unwrap();
        assert_eq!(hdd["status"], "failing");
        assert_eq!(hdd["power_on_hours"], 20000);
        assert_eq!(hdd["temperature_c"], 36);
        let usb = d
            .iter()
            .find(|x| x["name"] == "SanDisk Cruzer Blade")
            .unwrap();
        assert_eq!(usb["status"], "unknown");
        assert_eq!(usb["removable"], true);
    }

    #[test]
    fn windows_array_and_single() {
        let d = windows_disks(&fx("windows_physicaldisk.json")).unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!(d[0]["status"], "good");
        assert_eq!(d[0]["media"], "ssd");
        assert_eq!(d[0]["temperature_c"], 41);
        assert_eq!(d[1]["status"], "warning");
        assert_eq!(d[1]["media"], "hdd");
        let one = windows_disks(&fx("windows_physicaldisk_single.json")).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0]["media"], Value::Null);
        assert_eq!(one[0]["temperature_c"], Value::Null);
        assert!(windows_disks("").unwrap().is_empty());
        assert!(windows_disks("oops").is_err());
    }

    #[test]
    fn reads_this_machine() {
        let h = health();
        eprintln!("{}", serde_json::to_string_pretty(&h).unwrap());
        assert!(h["disks"].is_array());
    }

    #[test]
    fn script_has_no_interpolation() {
        assert!(!WINDOWS_SCRIPT.contains("{}"));
        assert!(!WINDOWS_SCRIPT.contains("$env:"));
    }
}
