pub fn writes(name: &str) -> bool {
    matches!(
        name,
        "gd_browser_open"
            | "gd_click"
            | "gd_type"
            | "gd_press"
            | "gd_scroll"
            | "gd_navigate"
            | "gd_browser_import"
            | "gd_page_eval"
            | "gd_connector_scaffold"
            | "gd_connector_install_dev"
            | "gd_connector_options"
            | "gd_download"
    )
}

pub fn open_world(name: &str) -> bool {
    matches!(
        name,
        "gd_browser_open" | "gd_navigate" | "gd_connector_run" | "gd_download"
    )
}
