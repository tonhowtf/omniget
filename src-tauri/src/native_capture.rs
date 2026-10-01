#[cfg(target_os = "macos")]
pub async fn snapshot(window: &tauri::WebviewWindow) -> Result<Vec<u8>, String> {
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
    use objc2_foundation::{NSDictionary, NSError};
    use objc2_web_kit::WKWebView;
    use std::sync::{Arc, Mutex};
    let (tx, rx) = tokio::sync::oneshot::channel();
    let tx = Arc::new(Mutex::new(Some(tx)));
    window
        .with_webview(move |webview| unsafe {
            let view = &*(webview.inner() as *const WKWebView);
            let callback = block2::RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                let result = (|| {
                    if !error.is_null() || image.is_null() {
                        return Err("CAPTURE_FAILED: WebKit snapshot failed".to_string());
                    }
                    let tiff = (*image)
                        .TIFFRepresentation()
                        .ok_or("CAPTURE_FAILED: TIFF")?;
                    let bitmap = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &tiff)
                        .ok_or("CAPTURE_FAILED: bitmap")?;
                    let png = bitmap
                        .representationUsingType_properties(
                            NSBitmapImageFileType::PNG,
                            &NSDictionary::new(),
                        )
                        .ok_or("CAPTURE_FAILED: PNG")?;
                    let bytes = png.to_vec();
                    if bytes.len() > 32 * 1024 * 1024 {
                        return Err("CAPTURE_FAILED: image exceeds 32 MiB".into());
                    }
                    Ok(bytes)
                })();
                if let Some(sender) = tx.lock().unwrap().take() {
                    let _ = sender.send(result);
                }
            });
            view.takeSnapshotWithConfiguration_completionHandler(None, &callback);
        })
        .map_err(|e| format!("CAPTURE_FAILED: {e}"))?;
    tokio::time::timeout(std::time::Duration::from_secs(10), rx)
        .await
        .map_err(|_| "TIMEOUT: native snapshot".to_string())?
        .map_err(|_| "CAPTURE_FAILED: callback closed".to_string())?
}

#[cfg(not(target_os = "macos"))]
pub async fn snapshot(_: &tauri::WebviewWindow) -> Result<Vec<u8>, String> {
    Err("CAPABILITY_UNAVAILABLE: native capture is not implemented on this platform".into())
}
