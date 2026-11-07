const MANIFEST: &str = r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="OmniGet.Tools" version="1.0.0.0"/>
  <description>OmniGet command line and helper tools</description>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security><requestedPrivileges><requestedExecutionLevel level="asInvoker" uiAccess="false"/></requestedPrivileges></security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application><supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/></application>
  </compatibility>
</assembly>"#;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = tauri_winres::WindowsResource::new();
    res.set("ProductName", "OmniGet")
        .set("CompanyName", "OmniGet")
        .set("FileDescription", "OmniGet command line and helper tools")
        .set(
            "LegalCopyright",
            "Copyright (C) OmniGet contributors. GPL-3.0",
        )
        .set_icon("../icons/icon.ico")
        .set_manifest(MANIFEST);
    if let Err(error) = res.compile() {
        println!("cargo:warning=Windows file details not embedded: {error}");
    }
}
