//! Downloads › Tools › "Download with OmniGet" in the macOS Services menu.
//!
//! Idea from MacYTDL, which installs a Service so a link selected in any app
//! can be sent to it. This writes a small Automator Quick Action into
//! `~/Library/Services`: it takes the selected text, pulls out every web or
//! magnet link and opens each one as `omniget://<link>`, which the app
//! already handles (`external_url::normalize_external_url`). Nothing runs in
//! the background and nothing needs an administrator password; removing the
//! bundle removes the menu item.

use serde::Serialize;
#[cfg(target_os = "macos")]
use std::path::PathBuf;

#[cfg(target_os = "macos")]
const BUNDLE_NAME: &str = "Download with OmniGet.workflow";

/// Shell run by the Quick Action with the selected text as arguments.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const SCRIPT: &str = r#"for arg in "$@"; do printf '%s\n' "$arg"; done \
  | grep -Eo '(https?://|magnet:\?)[^[:space:]<>"]+' \
  | sed -E 's/[.,;:!?)]+$//' \
  | awk '!seen[$0]++' \
  | while IFS= read -r link; do /usr/bin/open "omniget://$link"; done"#;

#[derive(Debug, Clone, Serialize)]
pub struct QuickActionStatus {
    pub supported: bool,
    pub installed: bool,
    pub path: Option<String>,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn info_plist(menu_title: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>NSServices</key>
	<array>
		<dict>
			<key>NSMenuItem</key>
			<dict>
				<key>default</key>
				<string>{title}</string>
			</dict>
			<key>NSMessage</key>
			<string>runWorkflowAsService</string>
			<key>NSSendTypes</key>
			<array>
				<string>public.utf8-plain-text</string>
			</array>
		</dict>
	</array>
</dict>
</plist>
"#,
        title = xml_escape(menu_title)
    )
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn document_wflow() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>AMApplicationBuild</key>
	<string>528</string>
	<key>AMApplicationVersion</key>
	<string>2.10</string>
	<key>AMDocumentVersion</key>
	<string>2</string>
	<key>actions</key>
	<array>
		<dict>
			<key>action</key>
			<dict>
				<key>AMAccepts</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Optional</key>
					<true/>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>AMActionVersion</key>
				<string>2.0.3</string>
				<key>AMApplication</key>
				<array>
					<string>Automator</string>
				</array>
				<key>AMParameterProperties</key>
				<dict>
					<key>COMMAND_STRING</key>
					<dict/>
					<key>CheckedForUserDefaultShell</key>
					<dict/>
					<key>inputMethod</key>
					<dict/>
					<key>shell</key>
					<dict/>
					<key>source</key>
					<dict/>
				</dict>
				<key>AMProvides</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>ActionBundlePath</key>
				<string>/System/Library/Automator/Run Shell Script.action</string>
				<key>ActionName</key>
				<string>Run Shell Script</string>
				<key>ActionParameters</key>
				<dict>
					<key>COMMAND_STRING</key>
					<string>{script}</string>
					<key>CheckedForUserDefaultShell</key>
					<true/>
					<key>inputMethod</key>
					<integer>1</integer>
					<key>shell</key>
					<string>/bin/zsh</string>
					<key>source</key>
					<string></string>
				</dict>
				<key>BundleIdentifier</key>
				<string>com.apple.RunShellScript</string>
				<key>CFBundleVersion</key>
				<string>2.0.3</string>
				<key>CanShowSelectedItemsWhenRun</key>
				<false/>
				<key>CanShowWhenRun</key>
				<true/>
				<key>Category</key>
				<array>
					<string>AMCategoryUtilities</string>
				</array>
				<key>Class Name</key>
				<string>RunShellScriptAction</string>
				<key>InputUUID</key>
				<string>4E0E5B6B-2C1F-4D6A-9B7E-0B1F3C9D2A11</string>
				<key>Keywords</key>
				<array>
					<string>Shell</string>
					<string>Script</string>
				</array>
				<key>OutputUUID</key>
				<string>8C3F2A10-6D4B-4E5C-A1B2-3C4D5E6F7A81</string>
				<key>UUID</key>
				<string>1B2C3D4E-5F60-4718-9A0B-C1D2E3F4A5B6</string>
				<key>UnlocalizedApplications</key>
				<array>
					<string>Automator</string>
				</array>
				<key>arguments</key>
				<dict/>
				<key>isViewVisible</key>
				<integer>1</integer>
			</dict>
			<key>isViewVisible</key>
			<integer>1</integer>
		</dict>
	</array>
	<key>connectors</key>
	<dict/>
	<key>workflowMetaData</key>
	<dict>
		<key>applicationBundleIDsByPath</key>
		<dict/>
		<key>applicationPaths</key>
		<array/>
		<key>inputTypeIdentifier</key>
		<string>com.apple.Automator.text</string>
		<key>outputTypeIdentifier</key>
		<string>com.apple.Automator.nothing</string>
		<key>presentationMode</key>
		<integer>11</integer>
		<key>processesInput</key>
		<false/>
		<key>serviceInputTypeIdentifier</key>
		<string>com.apple.Automator.text</string>
		<key>serviceOutputTypeIdentifier</key>
		<string>com.apple.Automator.nothing</string>
		<key>serviceProcessesInput</key>
		<false/>
		<key>systemImageName</key>
		<string>NSActionTemplate</string>
		<key>useAutomaticInputType</key>
		<false/>
		<key>workflowTypeIdentifier</key>
		<string>com.apple.Automator.servicesMenu</string>
	</dict>
</dict>
</plist>
"#,
        script = xml_escape(SCRIPT)
    )
}

#[cfg(target_os = "macos")]
fn bundle_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Home folder not found".to_string())?;
    Ok(home.join("Library/Services").join(BUNDLE_NAME))
}

#[cfg(target_os = "macos")]
fn refresh_services() {
    // Asks the pasteboard server to rescan ~/Library/Services right away;
    // without it the item can take until the next login to show up.
    let _ = std::process::Command::new("/System/Library/CoreServices/pbs")
        .arg("-update")
        .status();
}

#[cfg(target_os = "macos")]
fn status() -> QuickActionStatus {
    let path = bundle_path().ok();
    let installed = path
        .as_ref()
        .is_some_and(|p| p.join("Contents/document.wflow").is_file());
    QuickActionStatus {
        supported: true,
        installed,
        path: path.map(|p| p.to_string_lossy().to_string()),
    }
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn quick_action_status() -> QuickActionStatus {
    status()
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn quick_action_install(menu_title: String) -> Result<QuickActionStatus, String> {
    let trimmed = menu_title.trim();
    let title = if trimmed.is_empty() {
        "Download with OmniGet"
    } else {
        trimmed
    };
    let contents = bundle_path()?.join("Contents");
    std::fs::create_dir_all(&contents).map_err(|e| e.to_string())?;
    std::fs::write(contents.join("Info.plist"), info_plist(title)).map_err(|e| e.to_string())?;
    std::fs::write(contents.join("document.wflow"), document_wflow()).map_err(|e| e.to_string())?;
    refresh_services();
    Ok(status())
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn quick_action_remove() -> Result<QuickActionStatus, String> {
    let root = bundle_path()?;
    if root.exists() {
        std::fs::remove_dir_all(&root).map_err(|e| e.to_string())?;
    }
    refresh_services();
    Ok(status())
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn quick_action_status() -> QuickActionStatus {
    QuickActionStatus {
        supported: false,
        installed: false,
        path: None,
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn quick_action_install(menu_title: String) -> Result<QuickActionStatus, String> {
    let _ = menu_title;
    Err("macOS only".to_string())
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn quick_action_remove() -> Result<QuickActionStatus, String> {
    Err("macOS only".to_string())
}
