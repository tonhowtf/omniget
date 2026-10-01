//! Built-in addresses of Telegram's production data centers. The same list
//! ships in Telegram Desktop and TDLib; `help.getConfig` refreshes it once
//! a session is up, but the first connection has to start from here.

use std::net::{IpAddr, SocketAddr};

#[derive(Debug, Clone, Copy)]
pub struct DcAddr {
    pub id: i16,
    pub ip: &'static str,
}

pub const DC_V4: &[DcAddr] = &[
    DcAddr {
        id: 1,
        ip: "149.154.175.50",
    },
    DcAddr {
        id: 2,
        ip: "149.154.167.51",
    },
    DcAddr {
        id: 2,
        ip: "95.161.76.100",
    },
    DcAddr {
        id: 3,
        ip: "149.154.175.100",
    },
    DcAddr {
        id: 4,
        ip: "149.154.167.91",
    },
    DcAddr {
        id: 5,
        ip: "149.154.171.5",
    },
];

pub const DC_V6: &[DcAddr] = &[
    DcAddr {
        id: 1,
        ip: "2001:b28:f23d:f001::a",
    },
    DcAddr {
        id: 2,
        ip: "2001:67c:4e8:f002::a",
    },
    DcAddr {
        id: 3,
        ip: "2001:b28:f23d:f003::a",
    },
    DcAddr {
        id: 4,
        ip: "2001:67c:4e8:f004::a",
    },
    DcAddr {
        id: 5,
        ip: "2001:b28:f23f:f005::a",
    },
];

pub const PORT: u16 = 443;

pub fn addr(dc: i16, ipv6: bool) -> SocketAddr {
    let table = if ipv6 { DC_V6 } else { DC_V4 };
    let ip = table
        .iter()
        .find(|d| d.id == dc.abs())
        .map(|d| d.ip)
        .unwrap_or(DC_V4[1].ip);
    SocketAddr::new(ip.parse().expect("static DC ip"), PORT)
}

/// Maps an address back to its DC. MTProto proxies need the DC number, not
/// the IP, and the IPs from `help.getConfig` share the known /24s; an
/// unknown address falls back to the closest known prefix, then DC 2.
pub fn dc_for(ip: IpAddr) -> i16 {
    let s = ip.to_string();
    if let Some(d) = DC_V4.iter().chain(DC_V6.iter()).find(|d| d.ip == s) {
        return d.id;
    }
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            match (o[0], o[1], o[2]) {
                (149, 154, 175) => {
                    if o[3] >= 100 {
                        3
                    } else {
                        1
                    }
                }
                (149, 154, 167) => {
                    if o[3] >= 90 {
                        4
                    } else {
                        2
                    }
                }
                (149, 154, 171) | (91, 108, 56) => 5,
                (91, 108, 4) | (91, 108, 8) => 4,
                (95, 161, 76) => 2,
                _ => 2,
            }
        }
        IpAddr::V6(v6) => {
            let seg = v6.segments();
            let last = (seg[3] & 0xf) as i16;
            if (1..=5).contains(&last) {
                last
            } else {
                2
            }
        }
    }
}
