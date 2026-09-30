// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use crate::defs::{DcOption, PeerId, PeerInfo, UpdateState, UpdatesState};

/// The main interface to interact with the different [`crate::storages`].
///
/// All methods are synchronous and currently infallible because clients
/// are not equipped to deal with the arbitrary errors that a dynamic
/// `Session` could produce. This may change in the future.
///
/// A newly-created storage should return the same values that
/// [crate::SessionData::default] would produce.
pub trait Session: Send + Sync {
    /// Datacenter that is "home" to the user authorized by this session.
    ///
    /// If not known, the ID of the closest datacenter should be returned instead.
    /// Note that incorrect guesses are allowed, and the user may need to migrate.
    ///
    /// This method should be cheap to call, because it is used on every request.
    fn home_dc_id(&self) -> i32;

    /// Changes the [`Session::home_dc_id`] after finding out the actual datacenter
    /// to which main queries should be executed against.
    fn set_home_dc_id(&self, dc_id: i32);

    /// Query a single datacenter option.
    ///
    /// If no up-to-date option has been [`Session::set_dc_option`] yet,
    /// a statically-known option must be returned.
    ///
    /// `None` may only be returned on invalid DC IDs or DCs that are not yet known.
    ///
    /// This method should be cheap to call, because it is used on every request.
    fn dc_option(&self, dc_id: i32) -> Option<DcOption>;

    /// Update the previously-known [`Session::dc_option`] with new values.
    ///
    /// Should also be used after generating permanent authentication keys to a datacenter.
    fn set_dc_option(&self, dc_option: &DcOption);

    /// Query a peer by its identity.
    ///
    /// Querying for [`PeerId::self_user`] can be used as a way to determine
    /// whether the authentication key has a logged-in user bound (i.e. signed in).
    fn peer(&self, peer: PeerId) -> Option<PeerInfo>;

    /// Cache a peer's basic information for [`Session::peer`] to be able to query them later.
    ///
    /// This method may not necessarily remember the peers forever,
    /// except for users where [`PeerInfo::User::is_self`] is `Some(true)`.
    fn cache_peer(&self, peer: &PeerInfo);

    /// Loads the entire updates state.
    fn updates_state(&self) -> UpdatesState;

    /// Update the state for one or all updates.
    fn set_update_state(&self, update: UpdateState);
}
