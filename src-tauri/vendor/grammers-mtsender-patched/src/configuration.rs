// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

const DEFAULT_LOCALE: &str = "en";

/// Connection parameters used whenever a new connection is initialized.
///
/// After creating a [`crate::SenderPool::with_configuration`], the connection of
/// any of the [`crate::Sender`]s that it uses internally will be initialized with
/// an instance of [`grammers_tl_types::functions::InitConnection`].
///
/// Some fields are hidden to encourage using the Struct Update Syntax with a default.
pub struct ConnectionParams {
    /// "Device model" according to [`initConnection`](https://core.telegram.org/method/initConnection).
    pub device_model: String,
    /// "Operation system version" according to [`initConnection`](https://core.telegram.org/method/initConnection).
    pub system_version: String,
    /// "Application version" according to [`initConnection`](https://core.telegram.org/method/initConnection).
    pub app_version: String,
    /// Code for the language used on the device's OS, formatted using the ISO 639-1 standard.
    pub system_lang_code: String,
    /// Either an ISO 639-1 language code or a language pack name obtained from
    /// a [language pack link](https://core.telegram.org/api/links#language-pack-links).
    pub lang_code: String,
    /// URL of the proxy to use. Requires the `proxy` feature to be enabled.
    ///
    /// The scheme must be `socks5`. Username and password are optional, e.g.:
    /// - socks5://127.0.0.1:1234
    /// - socks5://username:password@example.com:5678
    ///
    /// Both a host and port must be provided. If a domain is used for the host, its address will be looked up,
    /// and the first IP address found will be used. If a different IP address should be used, consider resolving
    /// the host manually and selecting an IP address of your choice.
    #[cfg(feature = "proxy")]
    pub proxy_url: Option<String>,
    #[doc(hidden)]
    pub __non_exhaustive: (),
}

impl Default for ConnectionParams {
    fn default() -> Self {
        let info = os_info::get();

        let mut system_lang_code = String::new();
        let mut lang_code = String::new();

        #[cfg(not(target_os = "android"))]
        {
            system_lang_code.push_str(&locate_locale::system());
            lang_code.push_str(&locate_locale::user());
        }
        if system_lang_code.is_empty() {
            system_lang_code.push_str(DEFAULT_LOCALE);
        }
        if lang_code.is_empty() {
            lang_code.push_str(DEFAULT_LOCALE);
        }

        Self {
            device_model: format!("{} {}", info.os_type(), info.bitness()),
            system_version: info.version().to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            system_lang_code,
            lang_code,
            #[cfg(feature = "proxy")]
            proxy_url: None,
            __non_exhaustive: (),
        }
    }
}
