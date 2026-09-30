// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::net::{Ipv4Addr, SocketAddrV4, SocketAddrV6};

use crate::defs::DcOption;

pub(crate) const DEFAULT_DC: i32 = 2;

const fn ipv4(a: u8, b: u8, c: u8, d: u8) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), 443)
}

const fn ipv6(a: u8, b: u8, c: u8, d: u8) -> SocketAddrV6 {
    SocketAddrV6::new(ipv4(a, b, c, d).ip().to_ipv6_compatible(), 443, 0, 0)
}

/// Hardcoded known `static` options from `functions::help::GetConfig`.
pub(crate) const KNOWN_DC_OPTIONS: [DcOption; 5] = [
    DcOption {
        id: 1,
        ipv4: ipv4(149, 154, 175, 53),
        ipv6: ipv6(149, 154, 175, 53),
        auth_key: None,
    },
    DcOption {
        id: 2,
        ipv4: ipv4(149, 154, 167, 41),
        ipv6: ipv6(149, 154, 167, 41),
        auth_key: None,
    },
    DcOption {
        id: 3,
        ipv4: ipv4(149, 154, 175, 100),
        ipv6: ipv6(149, 154, 175, 100),
        auth_key: None,
    },
    DcOption {
        id: 4,
        ipv4: ipv4(149, 154, 167, 92),
        ipv6: ipv6(149, 154, 167, 92),
        auth_key: None,
    },
    DcOption {
        id: 5,
        ipv4: ipv4(91, 108, 56, 104),
        ipv6: ipv6(91, 108, 56, 104),
        auth_key: None,
    },
];
