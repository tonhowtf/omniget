use anyhow::Result;
use std::path::PathBuf;

use crate::cookies;
use crate::output;

pub async fn execute(file: String, name: Option<String>, dry_run: bool) -> Result<()> {
    let path = PathBuf::from(&file);

    if dry_run {
        let preview = cookies::preview_import(&path)?;
        if output::is_json_mode() {
            let items: Vec<serde_json::Value> = preview
                .iter()
                .map(|(domain, count)| {
                    serde_json::json!({ "domain": domain, "cookies": count })
                })
                .collect();
            output::print_json(&serde_json::json!({ "domains": items }));
        } else {
            println!("Preview ({} domains):", preview.len());
            for (domain, count) in &preview {
                println!("  {:<40} {} cookies", domain, count);
            }
        }
        return Ok(());
    }

    let total = cookies::import_cookies_file(&path, name.as_deref())?;
    let dest = if let Some(n) = &name {
        cookies::default_cookie_file().map(|d| d.with_file_name(format!("cookies_{}.txt", n)))
    } else {
        cookies::default_cookie_file()
    };

    if output::is_json_mode() {
        println!(
            r#"{{"success":true,"total":{},"destination":"{}"}}"#,
            total,
            dest.as_deref().map(|p| p.display().to_string()).unwrap_or_default()
        );
    } else {
        if let Some(d) = &dest {
            println!("✓ Imported {} cookies → {}", total, d.display());
        } else {
            println!("✓ Imported {} cookies", total);
        }
        if let Some(n) = &name {
            println!("  Account name: {}", n);
        } else {
            println!("  (default account)");
        }
    }

    Ok(())
}
