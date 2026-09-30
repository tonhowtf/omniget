// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

// OmniGet: rewritten on `rusqlite` (the app already links SQLite through it,
// and two crates linking `sqlite3` cannot coexist). Same schema and public
// API as upstream 0.8, so session files written by the old plugin open.

use crate::defs::{
    ChannelKind, ChannelState, DcOption, PeerAuth, PeerId, PeerInfo, PeerKind, UpdateState,
    UpdatesState,
};
use crate::{DEFAULT_DC, KNOWN_DC_OPTIONS, Session};
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::path::Path;
use std::sync::Mutex;

const VERSION: i64 = 1;

/// SQLite-based storage. This is the recommended option.
pub struct SqliteSession {
    database: Mutex<Connection>,
}

#[repr(u8)]
enum PeerSubtype {
    UserSelf = 1,
    UserBot = 2,
    UserSelfBot = 3,
    Megagroup = 4,
    Broadcast = 8,
    Gigagroup = 12,
}

fn init(db: &Connection) -> rusqlite::Result<()> {
    let user_version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if user_version == VERSION {
        return Ok(());
    }
    if user_version == 0 {
        db.execute_batch(
            "BEGIN;
            CREATE TABLE dc_home (
                dc_id INTEGER NOT NULL,
                PRIMARY KEY(dc_id));
            CREATE TABLE dc_option (
                dc_id INTEGER NOT NULL,
                ipv4 TEXT NOT NULL,
                ipv6 TEXT NOT NULL,
                auth_key BLOB,
                PRIMARY KEY (dc_id));
            CREATE TABLE peer_info (
                peer_id INTEGER NOT NULL,
                hash INTEGER,
                subtype INTEGER,
                PRIMARY KEY (peer_id));
            CREATE TABLE update_state (
                pts INTEGER NOT NULL,
                qts INTEGER NOT NULL,
                date INTEGER NOT NULL,
                seq INTEGER NOT NULL);
            CREATE TABLE channel_state (
                peer_id INTEGER NOT NULL,
                pts INTEGER NOT NULL,
                PRIMARY KEY (peer_id));
            COMMIT;",
        )?;
    }
    // Can't bind PRAGMA parameters, but `VERSION` is not user-controlled input.
    db.execute_batch(&format!("PRAGMA user_version = {VERSION}"))?;
    Ok(())
}

impl SqliteSession {
    /// Open a connection to the SQLite database at `path`,
    /// creating one if it doesn't exist.
    pub fn open<P: AsRef<Path>>(path: P) -> rusqlite::Result<Self> {
        let database = Connection::open(path)?;
        init(&database)?;
        Ok(SqliteSession {
            database: Mutex::new(database),
        })
    }
}

fn channel_kind(subtype: Option<u8>) -> Option<ChannelKind> {
    subtype.and_then(|s| {
        if (s & PeerSubtype::Gigagroup as u8) == PeerSubtype::Gigagroup as u8 {
            Some(ChannelKind::Gigagroup)
        } else if s & PeerSubtype::Broadcast as u8 != 0 {
            Some(ChannelKind::Broadcast)
        } else if s & PeerSubtype::Megagroup as u8 != 0 {
            Some(ChannelKind::Megagroup)
        } else {
            None
        }
    })
}

impl Session for SqliteSession {
    fn home_dc_id(&self) -> i32 {
        let db = self.database.lock().unwrap();
        db.query_row("SELECT dc_id FROM dc_home LIMIT 1", [], |r| r.get::<_, i64>(0))
            .optional()
            .unwrap()
            .map(|v| v as i32)
            .unwrap_or(DEFAULT_DC)
    }

    fn set_home_dc_id(&self, dc_id: i32) {
        let mut db = self.database.lock().unwrap();
        let tx = db.transaction().unwrap();
        tx.execute("DELETE FROM dc_home", []).unwrap();
        tx.execute("INSERT INTO dc_home VALUES (?1)", params![dc_id as i64])
            .unwrap();
        tx.commit().unwrap();
    }

    fn dc_option(&self, dc_id: i32) -> Option<DcOption> {
        let db = self.database.lock().unwrap();
        db.query_row(
            "SELECT dc_id, ipv4, ipv6, auth_key FROM dc_option WHERE dc_id = ?1 LIMIT 1",
            params![dc_id as i64],
            |r| {
                let key: Option<Vec<u8>> = r.get(3)?;
                Ok(DcOption {
                    id: r.get::<_, i64>(0)? as _,
                    ipv4: r.get::<_, String>(1)?.parse().unwrap(),
                    ipv6: r.get::<_, String>(2)?.parse().unwrap(),
                    auth_key: key.map(|k| k.try_into().unwrap()),
                })
            },
        )
        .optional()
        .unwrap()
        .or_else(|| {
            KNOWN_DC_OPTIONS
                .iter()
                .find(|dc_option| dc_option.id == dc_id)
                .cloned()
        })
    }

    fn set_dc_option(&self, dc_option: &DcOption) {
        let db = self.database.lock().unwrap();
        db.execute(
            "INSERT OR REPLACE INTO dc_option VALUES (?1, ?2, ?3, ?4)",
            params![
                dc_option.id as i64,
                dc_option.ipv4.to_string(),
                dc_option.ipv6.to_string(),
                dc_option.auth_key.as_ref().map(|k| k.to_vec()),
            ],
        )
        .unwrap();
    }

    fn peer(&self, peer: PeerId) -> Option<PeerInfo> {
        let db = self.database.lock().unwrap();
        let map_row = |r: &Row| -> rusqlite::Result<PeerInfo> {
            let peer_id: i64 = r.get(0)?;
            let hash: Option<i64> = r.get(1)?;
            let subtype = r.get::<_, Option<i64>>(2)?.map(|s| s as u8);
            Ok(match peer.kind() {
                PeerKind::User | PeerKind::UserSelf => PeerInfo::User {
                    id: PeerId::user(peer_id).bare_id(),
                    auth: hash.map(PeerAuth::from_hash),
                    bot: subtype.map(|s| s & PeerSubtype::UserBot as u8 != 0),
                    is_self: subtype.map(|s| s & PeerSubtype::UserSelf as u8 != 0),
                },
                PeerKind::Chat => PeerInfo::Chat { id: peer.bare_id() },
                PeerKind::Channel => PeerInfo::Channel {
                    id: peer.bare_id(),
                    auth: hash.map(PeerAuth::from_hash),
                    kind: channel_kind(subtype),
                },
            })
        };
        if peer.kind() == PeerKind::UserSelf {
            db.query_row(
                "SELECT peer_id, hash, subtype FROM peer_info WHERE subtype & ?1 LIMIT 1",
                params![PeerSubtype::UserSelf as i64],
                map_row,
            )
            .optional()
            .unwrap()
        } else {
            db.query_row(
                "SELECT peer_id, hash, subtype FROM peer_info WHERE peer_id = ?1 LIMIT 1",
                params![peer.bot_api_dialog_id()],
                map_row,
            )
            .optional()
            .unwrap()
        }
    }

    fn cache_peer(&self, peer: &PeerInfo) {
        let db = self.database.lock().unwrap();
        let hash = (peer.auth() != PeerAuth::default()).then(|| peer.auth().hash());
        let subtype = match peer {
            PeerInfo::User { bot, is_self, .. } => {
                match (bot.unwrap_or_default(), is_self.unwrap_or_default()) {
                    (true, true) => Some(PeerSubtype::UserSelfBot),
                    (true, false) => Some(PeerSubtype::UserBot),
                    (false, true) => Some(PeerSubtype::UserSelf),
                    (false, false) => None,
                }
            }
            PeerInfo::Chat { .. } => None,
            PeerInfo::Channel { kind, .. } => kind.map(|kind| match kind {
                ChannelKind::Megagroup => PeerSubtype::Megagroup,
                ChannelKind::Broadcast => PeerSubtype::Broadcast,
                ChannelKind::Gigagroup => PeerSubtype::Gigagroup,
            }),
        };
        db.execute(
            "INSERT OR REPLACE INTO peer_info VALUES (?1, ?2, ?3)",
            params![
                peer.id().bot_api_dialog_id(),
                hash,
                subtype.map(|s| s as i64)
            ],
        )
        .unwrap();
    }

    fn updates_state(&self) -> UpdatesState {
        let db = self.database.lock().unwrap();
        let mut state = db
            .query_row(
                "SELECT pts, qts, date, seq FROM update_state LIMIT 1",
                [],
                |r| {
                    Ok(UpdatesState {
                        pts: r.get::<_, i64>(0)? as _,
                        qts: r.get::<_, i64>(1)? as _,
                        date: r.get::<_, i64>(2)? as _,
                        seq: r.get::<_, i64>(3)? as _,
                        channels: Vec::new(),
                    })
                },
            )
            .optional()
            .unwrap()
            .unwrap_or_default();
        let mut stmt = db
            .prepare("SELECT peer_id, pts FROM channel_state")
            .unwrap();
        state.channels = stmt
            .query_map([], |r| {
                Ok(ChannelState {
                    id: r.get(0)?,
                    pts: r.get::<_, i64>(1)? as _,
                })
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        state
    }

    fn set_update_state(&self, update: UpdateState) {
        let mut db = self.database.lock().unwrap();
        let tx = db.transaction().unwrap();
        let has_row = |tx: &rusqlite::Transaction| -> bool {
            tx.query_row("SELECT 1 FROM update_state LIMIT 1", [], |_| Ok(()))
                .optional()
                .unwrap()
                .is_some()
        };
        match update {
            UpdateState::All(updates_state) => {
                tx.execute("DELETE FROM update_state", []).unwrap();
                tx.execute(
                    "INSERT INTO update_state VALUES (?1, ?2, ?3, ?4)",
                    params![
                        updates_state.pts as i64,
                        updates_state.qts as i64,
                        updates_state.date as i64,
                        updates_state.seq as i64
                    ],
                )
                .unwrap();
                tx.execute("DELETE FROM channel_state", []).unwrap();
                for channel in updates_state.channels {
                    tx.execute(
                        "INSERT INTO channel_state VALUES (?1, ?2)",
                        params![channel.id as i64, channel.pts as i64],
                    )
                    .unwrap();
                }
            }
            UpdateState::Primary { pts, date, seq } => {
                if has_row(&tx) {
                    tx.execute(
                        "UPDATE update_state SET pts = ?1, date = ?2, seq = ?3",
                        params![pts as i64, date as i64, seq as i64],
                    )
                    .unwrap();
                } else {
                    tx.execute(
                        "INSERT INTO update_state VALUES (?1, 0, ?2, ?3)",
                        params![pts as i64, date as i64, seq as i64],
                    )
                    .unwrap();
                }
            }
            UpdateState::Secondary { qts } => {
                if has_row(&tx) {
                    tx.execute("UPDATE update_state SET qts = ?1", params![qts as i64])
                        .unwrap();
                } else {
                    tx.execute(
                        "INSERT INTO update_state VALUES (0, ?1, 0, 0)",
                        params![qts as i64],
                    )
                    .unwrap();
                }
            }
            UpdateState::Channel { id, pts } => {
                tx.execute(
                    "INSERT OR REPLACE INTO channel_state VALUES (?1, ?2)",
                    params![id, pts as i64],
                )
                .unwrap();
            }
        }
        tx.commit().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};

    use {DcOption, KNOWN_DC_OPTIONS, PeerInfo, Session, UpdateState};

    use super::*;

    #[test]
    fn exercise_sqlite_session() {
        let session = SqliteSession::open(":memory:").unwrap();

        assert_eq!(session.home_dc_id(), DEFAULT_DC);
        session.set_home_dc_id(DEFAULT_DC + 1);
        assert_eq!(session.home_dc_id(), DEFAULT_DC + 1);

        assert_eq!(
            session.dc_option(KNOWN_DC_OPTIONS[0].id),
            Some(KNOWN_DC_OPTIONS[0].clone())
        );
        let new_dc_option = DcOption {
            id: KNOWN_DC_OPTIONS
                .iter()
                .map(|dc_option| dc_option.id)
                .max()
                .unwrap()
                + 1,
            ipv4: SocketAddrV4::new(Ipv4Addr::from_bits(0), 1),
            ipv6: SocketAddrV6::new(Ipv6Addr::from_bits(0), 1, 0, 0),
            auth_key: Some([1; 256]),
        };
        assert_eq!(session.dc_option(new_dc_option.id), None);
        session.set_dc_option(&new_dc_option);
        assert_eq!(session.dc_option(new_dc_option.id), Some(new_dc_option));

        assert_eq!(session.peer(PeerId::self_user()), None);
        assert_eq!(session.peer(PeerId::user(1)), None);
        let peer = PeerInfo::User {
            id: 1,
            auth: None,
            bot: Some(true),
            is_self: Some(true),
        };
        session.cache_peer(&peer);
        assert_eq!(session.peer(PeerId::self_user()), Some(peer.clone()));
        assert_eq!(session.peer(PeerId::user(1)), Some(peer));

        assert_eq!(session.peer(PeerId::channel(1)), None);
        let peer = PeerInfo::Channel {
            id: 1,
            auth: Some(PeerAuth::from_hash(-1)),
            kind: Some(ChannelKind::Broadcast),
        };
        session.cache_peer(&peer);
        assert_eq!(session.peer(PeerId::channel(1)), Some(peer));

        assert_eq!(session.updates_state(), UpdatesState::default());
        session.set_update_state(UpdateState::All(UpdatesState {
            pts: 1,
            qts: 2,
            date: 3,
            seq: 4,
            channels: vec![
                ChannelState { id: 5, pts: 6 },
                ChannelState { id: 7, pts: 8 },
            ],
        }));
        session.set_update_state(UpdateState::Primary {
            pts: 2,
            date: 4,
            seq: 5,
        });
        session.set_update_state(UpdateState::Secondary { qts: 3 });
        session.set_update_state(UpdateState::Channel { id: 7, pts: 9 });
        assert_eq!(
            session.updates_state(),
            UpdatesState {
                pts: 2,
                qts: 3,
                date: 4,
                seq: 5,
                channels: vec![
                    ChannelState { id: 5, pts: 6 },
                    ChannelState { id: 7, pts: 9 },
                ],
            }
        );
    }
}
