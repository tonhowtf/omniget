pub fn writes(name: &str) -> bool {
    matches!(
        name,
        "gd_browser_open"
            | "gd_click"
            | "gd_type"
            | "gd_press"
            | "gd_scroll"
            | "gd_navigate"
            | "gd_act"
            | "gd_cdp"
            | "gd_browser_attach"
            | "gd_browser_import"
            | "gd_page_eval"
            | "gd_connector_scaffold"
            | "gd_connector_install_dev"
            | "gd_connector_options"
            | "gd_market_install"
            | "gd_download"
    )
}

pub fn open_world(name: &str) -> bool {
    matches!(
        name,
        "gd_browser_open"
            | "gd_navigate"
            | "gd_act"
            | "gd_cdp"
            | "gd_connector_run"
            | "gd_market"
            | "gd_market_install"
            | "gd_download"
    )
}
