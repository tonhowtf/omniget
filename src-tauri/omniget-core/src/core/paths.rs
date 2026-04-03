pub fn app_data_dir() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("OMNIGET_DATA_DIR") {
        return Some(std::path::PathBuf::from(dir));
    }

    let base = dirs::data_dir()?;
    let new_path = base.join("wtf.tonho.omniget");
    let old_path = base.join("omniget");

    if old_path.exists() {
        let _ = std::fs::create_dir_all(&new_path);

        for dir_name in &["bin", "plugins"] {
            let src = old_path.join(dir_name);
            let dst = new_path.join(dir_name);
            if src.exists() && !dst.exists() {
                let _ = copy_dir_recursive(&src, &dst);
            }
        }

        if let Ok(entries) = std::fs::read_dir(&old_path) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    let dest = new_path.join(entry.file_name());
                    if !dest.exists() {
                        let _ = std::fs::copy(entry.path(), &dest);
                    }
                }
            }
        }
    }

    Some(new_path)
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let dest = dst.join(entry.file_name());
            copy_dir_recursive(&entry.path(), &dest)?;
        }
    } else {
        std::fs::copy(src, dst)?;
    }
    Ok(())
}
