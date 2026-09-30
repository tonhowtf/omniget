// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::collections::HashMap;

use crate::defs::{DcOption, PeerId, PeerInfo, UpdateState, UpdatesState};
use crate::{DEFAULT_DC, KNOWN_DC_OPTIONS, Session};

/// In-memory representation of the entire [`Session`] state.
///
/// This type can be used for conversions `From` any [`Session`],
/// and be [`SessionData::import_to`] any other [`Session`].
pub struct SessionData {
    /// The identifier of the datacenter option determined
    /// to be the primary one for the logged-in user, or
    /// the identifier of an arbitrary datacenter otherwise.
    pub home_dc: i32,
    /// List of all known datacenter options, along with their
    /// Authorization Key if an encrypted connection has been
    /// made to them previously. Indexed by their identifier.
    pub dc_options: HashMap<i32, DcOption>,
    /// List of all peer informations cached in the session.
    /// Indexed by their identifier.
    pub peer_infos: HashMap<PeerId, PeerInfo>,
    /// Entirety of the update state for the logged-in user.
    pub updates_state: UpdatesState,
}

impl Default for SessionData {
    /// Constructs a default instance of the session data, with an arbitrary
    /// [`Self::home_dc`] and the list of statically-known [`Self::dc_options`].
    fn default() -> Self {
        Self {
            home_dc: DEFAULT_DC,
            dc_options: KNOWN_DC_OPTIONS
                .iter()
                .cloned()
                .map(|dc_option| (dc_option.id, dc_option))
                .collect(),
            peer_infos: HashMap::new(),
            updates_state: UpdatesState::default(),
        }
    }
}

impl SessionData {
    /// Imports all information from this session data to a type implementing `Session`.
    pub fn import_to<S: Session>(&self, session: &S) {
        session.set_home_dc_id(self.home_dc);
        self.dc_options
            .values()
            .for_each(|dc_option| session.set_dc_option(dc_option));
        self.peer_infos
            .values()
            .for_each(|peer| session.cache_peer(peer));
        session.set_update_state(UpdateState::All(self.updates_state.clone()));
    }
}

impl<S: Session> From<S> for SessionData {
    /// Imports the basic information from any type implementing `Session` into `SessionData`.
    ///
    /// Notably, only the standard DC options and the cached self-peer will be included.
    fn from(session: S) -> Self {
        let home_dc = session.home_dc_id();
        let dc_options = KNOWN_DC_OPTIONS
            .iter()
            .map(|dc_option| (dc_option.id, session.dc_option(dc_option.id).unwrap()))
            .collect();
        let peer_infos = [session
            .peer(PeerId::self_user())
            .map(|peer_info| (peer_info.id(), peer_info))]
        .into_iter()
        .collect::<Option<_>>()
        .unwrap_or_default();
        let updates_state = session.updates_state();

        Self {
            home_dc,
            dc_options,
            peer_infos,
            updates_state,
        }
    }
}
