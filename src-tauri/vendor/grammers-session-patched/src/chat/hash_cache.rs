// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![allow(deprecated)]
use grammers_tl_types as tl;
use std::collections::HashMap;

use crate::defs::{PeerAuth, PeerId, PeerInfo, PeerRef};

/// In-memory chat cache, mapping peers to their respective access hashes.
#[deprecated(note = "Use the Session::peer instead")]
pub struct PeerAuthCache {
    hash_map: HashMap<PeerId, PeerAuth>,
    self_id: Option<i64>,
    self_bot: bool,
}

impl PeerAuthCache {
    pub fn new(self_user: Option<(i64, bool)>) -> Self {
        Self {
            hash_map: HashMap::new(),
            self_id: self_user.map(|user| user.0),
            self_bot: self_user.map(|user| user.1).unwrap_or(false),
        }
    }

    pub fn self_id(&self) -> i64 {
        self.self_id
            .expect("tried to query self_id before it's known")
    }

    pub fn is_self_bot(&self) -> bool {
        self.self_bot
    }

    pub fn set_self_user(&mut self, user: PeerInfo) {
        match user {
            PeerInfo::User { id, bot, .. } => {
                self.self_bot = bot.unwrap_or_default();
                self.self_id = Some(id);
            }
            _ => panic!("tried to set self-user without providing user type"),
        }
    }

    pub fn get(&self, id: PeerId) -> Option<PeerRef> {
        self.hash_map.get(&id).map(|&auth| PeerRef { id, auth })
    }

    // Returns `true` if all users and chats could be extended without issue.
    // Returns `false` if there is any user or chat for which its `access_hash` is missing.
    #[must_use]
    pub fn extend(&mut self, users: &[tl::enums::User], chats: &[tl::enums::Chat]) -> bool {
        // See https://core.telegram.org/api/min for "issues" with "min constructors".
        use tl::enums::{Chat as C, User as U};

        let mut success = true;

        users.iter().for_each(|user| match user {
            U::Empty(_) => {}
            U::User(u) => match (u.min, u.access_hash) {
                (false, Some(hash)) => {
                    self.hash_map
                        .insert(PeerId::user(u.id), PeerAuth::from_hash(hash));
                }
                _ => success &= self.hash_map.contains_key(&PeerId::user(u.id)),
            },
        });

        chats.iter().for_each(|chat| match chat {
            C::Empty(_) | C::Chat(_) | C::Forbidden(_) => {}
            C::Channel(c) => match (c.min, c.access_hash) {
                (false, Some(hash)) => {
                    self.hash_map
                        .insert(PeerId::channel(c.id), PeerAuth::from_hash(hash));
                }
                _ => success &= self.hash_map.contains_key(&PeerId::channel(c.id)),
            },
            C::ChannelForbidden(c) => {
                self.hash_map
                    .insert(PeerId::channel(c.id), PeerAuth::from_hash(c.access_hash));
            }
        });

        success
    }
}
