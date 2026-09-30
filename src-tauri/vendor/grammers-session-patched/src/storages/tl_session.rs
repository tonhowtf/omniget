// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![allow(deprecated)]

use crate::dc_options::DEFAULT_DC;
use crate::defs::PeerAuth;
use crate::generated::{enums, types};
use crate::{KNOWN_DC_OPTIONS, Session};
use grammers_tl_types::deserialize::Error as DeserializeError;
use grammers_tl_types::{Deserializable, Serializable};
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};
use std::path::Path;
use std::sync::Mutex;

/// Original session storage.
///
/// This storage leverages Telegram's own serialization format
/// to persist data to a file on-disk.
#[cfg_attr(
    feature = "impl-serde",
    derive(serde_derive::Serialize, serde_derive::Deserialize)
)]
#[deprecated(note = "Migrate to a different storage")]
pub struct TlSession {
    session: Mutex<types::Session>,
}

#[allow(clippy::new_without_default)]
impl TlSession {
    pub fn new() -> Self {
        let this = Self {
            session: Mutex::new(types::Session {
                dcs: Vec::with_capacity(KNOWN_DC_OPTIONS.len()),
                user: None,
                state: None,
            }),
        };
        KNOWN_DC_OPTIONS
            .iter()
            .for_each(|dc_option| this.set_dc_option(dc_option));
        this
    }

    /// Load a previous session instance from a file,
    /// creating one if it doesn't exist
    pub fn load_file_or_create<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            File::create(path)?;
            let session = TlSession::new();
            session.save_to_file(path)?;
            Ok(session)
        } else {
            Self::load_file(path)
        }
    }

    /// Load a previous session instance from a file.
    pub fn load_file<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let mut data = Vec::new();
        File::open(path.as_ref())?.read_to_end(&mut data)?;

        Self::load(&data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    pub fn load(data: &[u8]) -> Result<Self, Error> {
        let this = Self {
            session: Mutex::new(
                enums::Session::from_bytes(data)
                    .map_err(|e| match e {
                        DeserializeError::UnexpectedEof => Error::MalformedData,
                        DeserializeError::UnexpectedConstructor { .. } => Error::UnsupportedVersion,
                    })?
                    .into(),
            ),
        };
        KNOWN_DC_OPTIONS.iter().for_each(|dc_option| {
            if this.dc_option(dc_option.id).is_none() {
                this.set_dc_option(dc_option);
            }
        });
        Ok(this)
    }

    #[must_use]
    pub fn save(&self) -> Vec<u8> {
        enums::Session::Session(self.session.lock().unwrap().clone()).to_bytes()
    }

    /// Saves the session to a file.
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let mut file = OpenOptions::new().write(true).open(path.as_ref())?;
        file.seek(io::SeekFrom::Start(0))?;
        file.set_len(0)?;
        file.write_all(&self.save())?;
        file.sync_data()
    }
}

impl crate::Session for TlSession {
    fn home_dc_id(&self) -> i32 {
        let session = self.session.lock().unwrap();
        session
            .user
            .as_ref()
            .map(|enums::User::User(user)| user.dc)
            .unwrap_or(DEFAULT_DC)
    }

    fn set_home_dc_id(&self, dc_id: i32) {
        let mut session = self.session.lock().unwrap();
        if let Some(enums::User::User(user)) = &mut session.user {
            user.dc = dc_id
        } else {
            session.user = Some(enums::User::User(types::User {
                id: 0,
                bot: false,
                dc: dc_id,
            }))
        }
    }

    fn dc_option(&self, dc_id: i32) -> Option<crate::defs::DcOption> {
        let session = self.session.lock().unwrap();
        session.dcs.iter().find_map(|dc| match dc {
            enums::DataCenter::Center(center) if center.id == dc_id => {
                Some(crate::defs::DcOption {
                    id: center.id,
                    ipv4: SocketAddrV4::new(
                        Ipv4Addr::from_bits(center.ipv4.unwrap() as _),
                        center.port as _,
                    ),
                    ipv6: SocketAddrV6::new(
                        center
                            .ipv6
                            .map(|ipv6| Ipv6Addr::from_bits(u128::from_le_bytes(ipv6)))
                            .unwrap_or_else(|| {
                                Ipv4Addr::from_bits(center.ipv4.unwrap() as _).to_ipv6_mapped()
                            }),
                        center.port as _,
                        0,
                        0,
                    ),
                    auth_key: center.auth.as_deref().map(|auth| auth.try_into().unwrap()),
                })
            }
            _ => None,
        })
    }

    fn set_dc_option(&self, dc_option: &crate::defs::DcOption) {
        let mut session = self.session.lock().unwrap();

        if let Some(pos) = session.dcs.iter().position(|dc| dc.id() == dc_option.id) {
            session.dcs.remove(pos);
        }
        session
            .dcs
            .push(enums::DataCenter::Center(types::DataCenter {
                id: dc_option.id,
                ipv4: Some(dc_option.ipv4.ip().to_bits() as _),
                ipv6: Some(dc_option.ipv6.ip().to_bits().to_le_bytes()),
                port: dc_option.ipv4.port() as _,
                auth: dc_option.auth_key.map(|auth| auth.to_vec()),
            }));
    }

    fn peer(&self, peer: crate::defs::PeerId) -> Option<crate::defs::PeerInfo> {
        let session = self.session.lock().unwrap();
        if peer.kind() == crate::defs::PeerKind::UserSelf {
            session
                .user
                .as_ref()
                .map(|enums::User::User(user)| crate::defs::PeerInfo::User {
                    id: user.id,
                    auth: Some(PeerAuth::default()),
                    bot: Some(user.bot),
                    is_self: Some(true),
                })
        } else {
            None
        }
    }

    fn cache_peer(&self, peer: &crate::defs::PeerInfo) {
        let mut session = self.session.lock().unwrap();
        match peer {
            crate::defs::PeerInfo::User {
                id,
                auth: _,
                bot,
                is_self,
            } if *is_self == Some(true) => {
                if let Some(enums::User::User(user)) = &mut session.user {
                    user.id = *id;
                    user.bot = bot.unwrap_or_default();
                } else {
                    session.user = Some(enums::User::User(types::User {
                        id: *id,
                        bot: bot.unwrap_or_default(),
                        dc: DEFAULT_DC,
                    }))
                }
            }
            _ => {}
        }
    }

    fn updates_state(&self) -> crate::defs::UpdatesState {
        let session = self.session.lock().unwrap();
        session
            .state
            .as_ref()
            .map(
                |enums::UpdateState::State(state)| crate::defs::UpdatesState {
                    pts: state.pts,
                    qts: state.qts,
                    date: state.date,
                    seq: state.seq,
                    channels: state
                        .channels
                        .iter()
                        .map(
                            |enums::ChannelState::State(channel)| crate::defs::ChannelState {
                                id: channel.channel_id,
                                pts: channel.pts,
                            },
                        )
                        .collect(),
                },
            )
            .unwrap_or_default()
    }

    fn set_update_state(&self, update: crate::defs::UpdateState) {
        match update {
            crate::defs::UpdateState::All(updates_state) => {
                let mut session = self.session.lock().unwrap();
                session.state = Some(
                    types::UpdateState {
                        pts: updates_state.pts,
                        qts: updates_state.qts,
                        date: updates_state.date,
                        seq: updates_state.seq,
                        channels: updates_state
                            .channels
                            .iter()
                            .map(|channel| {
                                types::ChannelState {
                                    channel_id: channel.id,
                                    pts: channel.pts,
                                }
                                .into()
                            })
                            .collect(),
                    }
                    .into(),
                )
            }
            crate::defs::UpdateState::Primary { pts, date, seq } => {
                let mut current = self.updates_state();
                current.pts = pts;
                current.date = date;
                current.seq = seq;
                self.set_update_state(crate::defs::UpdateState::All(current));
            }
            crate::defs::UpdateState::Secondary { qts } => {
                let mut current = self.updates_state();
                current.qts = qts;
                self.set_update_state(crate::defs::UpdateState::All(current));
            }
            crate::defs::UpdateState::Channel { id, pts } => {
                let mut current = self.updates_state();
                if let Some(pos) = current.channels.iter().position(|channel| channel.id == id) {
                    current.channels[pos] = crate::defs::ChannelState { id: id, pts }
                } else {
                    current
                        .channels
                        .push(crate::defs::ChannelState { id: id, pts });
                }
                self.set_update_state(crate::defs::UpdateState::All(current));
            }
        }
    }
}

#[derive(Debug)]
pub enum Error {
    MalformedData,
    UnsupportedVersion,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::MalformedData => write!(f, "malformed data"),
            Error::UnsupportedVersion => write!(f, "unsupported version"),
        }
    }
}

impl std::error::Error for Error {}
