// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
#[cfg(test)]
use super::tests::Instant;
use grammers_tl_types as tl;
use std::time::Duration;
#[cfg(not(test))]
use std::time::Instant;

/// Telegram sends `seq` equal to `0` when "it doesn't matter", so we use that value too.
pub(super) const NO_SEQ: i32 = 0;

/// It has been observed that Telegram may send updates with `qts` equal to `0` (for
/// example with `ChannelParticipant`), interleaved with non-zero `qts` values. This
/// presumably means that the ordering should be "ignored" in that case.
///
/// One can speculate this is done because the field is not optional in the TL definition.
///
/// Not ignoring the `pts` information in those updates can lead to failures resolving gaps.
pub(super) const NO_PTS: i32 = 0;

/// Non-update types like `messages.affectedMessages` can contain `pts` that should still be
/// processed. Because there's no `date`, a value of `0` is used as the sentinel value for
/// the `date` when constructing the dummy `Updates` (in order to handle them uniformly).
pub(super) const NO_DATE: i32 = 0;

// > It may be useful to wait up to 0.5 seconds
pub(super) const POSSIBLE_GAP_TIMEOUT: Duration = Duration::from_millis(500);

/// After how long without updates the client will "timeout".
///
/// When this timeout occurs, the client will attempt to fetch updates by itself, ignoring all the
/// updates that arrive in the meantime. After all updates are fetched when this happens, the
/// client will resume normal operation, and the timeout will reset.
///
/// Documentation recommends 15 minutes without updates (https://core.telegram.org/api/updates).
pub(super) const NO_UPDATES_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// A sortable [`MessageBox`] entry key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Key {
    Common,
    Secondary,
    Channel(i64),
}

/// A live [`MessageBox`] entry.
#[derive(Debug)]
pub(super) struct LiveEntry {
    /// The variant of the [`MessageBox`] that this entry represents.
    pub(super) key: Key,

    /// The local persistent timestamp value that this [`MessageBox`] has.
    pub(super) pts: i32,

    /// Next instant when we would get the update difference if no updates arrived before then.
    pub(super) deadline: Instant,

    /// If the entry has a gap and may soon trigger the need to get difference.
    pub(super) possible_gap: Option<PossibleGap>,
}

/// Contains all live message boxes and is able to process incoming updates for each of them.
///
/// See <https://core.telegram.org/api/updates#message-related-event-sequences>.
#[derive(Debug)]
pub struct MessageBoxes {
    /// Live entries, sorted by key.
    pub(super) entries: Vec<LiveEntry>,

    /// Common [`State`] fields.
    pub(super) date: i32,
    pub(super) seq: i32,

    /// Optimization field to quickly query all entries that are currently being fetched.
    pub(super) getting_diff_for: Vec<Key>,

    /// Optimization field to quickly query all entries that have a possible gap.
    pub(super) possible_gaps: Vec<Key>,

    /// Optimization field holding the closest deadline instant.
    pub(super) next_deadline: Instant,
}

/// Represents the information needed to correctly handle a specific `tl::enums::Update`.
#[derive(Debug)]
pub(super) struct PtsInfo {
    pub(super) key: Key,
    pub(super) pts: i32,
    pub(super) count: i32,
}

// > ### Recovering gaps
// > […] Manually obtaining updates is also required in the following situations:
// > • Loss of sync: a gap was found in `seq` / `pts` / `qts` (as described above).
// >   It may be useful to wait up to 0.5 seconds in this situation and abort the sync in case a new update
// >   arrives, that fills the gap.
//
// This is really easy to trigger by spamming messages in a channel (with as little as 3 members works), because
// the updates produced by the RPC request take a while to arrive (whereas the read update comes faster alone).
#[derive(Debug)]
pub(super) struct PossibleGap {
    pub(super) deadline: Instant,
    /// Pending updates (those with a larger PTS, producing the gap which may later be filled).
    pub(super) updates: Vec<tl::enums::Update>,
}

/// Error when a gap has been detected during update processing.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Gap;

/// Alias for the commonly-referenced three-tuple of update and related peers.
pub(super) type UpdateAndPeers = (
    Vec<(tl::enums::Update, State)>,
    Vec<tl::enums::User>,
    Vec<tl::enums::Chat>,
);

/// Anything that should be treated like an update.
///
/// Telegram unfortunately also includes update state in types
/// that are not part of the usual [`tl::enums::Updates`].
#[derive(Debug)]
pub enum UpdatesLike {
    /// The usual variant, received passively from Telegram.
    Updates(tl::enums::Updates),
    /// Special-case for short-sent messages,
    /// where the request is also needed to construct a complete update.
    ShortSentMessage {
        /// The request that triggered the short update.
        request: tl::functions::messages::SendMessage,
        /// The incomplete update caused by the request.
        update: tl::types::UpdateShortSentMessage,
    },
    /// Special-case for requests that affect some messages.
    AffectedMessages(tl::types::messages::AffectedMessages),
    /// Special-case for requests that lead to users being invited.
    InvitedUsers(tl::types::messages::InvitedUsers),
}

// Public interface around the more tightly-packed internal state.

/// Update state, up to and including the update it is a part of.
///
/// When using `catch_up` with a client, all updates with a
/// state containing a [`MessageBox`] higher than this one will be fetched.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct State {
    /// Last date state assigned by Telegram, which changes somewhat arbitrarily.
    pub date: i32,
    /// Last sequence state assigned by Telegram, which changes somewhat arbitrarily.
    pub seq: i32,
    /// The particular message box change if the update pertains to a message-related event sequence.
    pub message_box: Option<MessageBox>,
}

/// The message box and pts value that uniquely identifies the message-related update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessageBox {
    /// Account-wide persistent timestamp.
    ///
    /// This includes private conversations (one-to-one) and small group chats.
    Common { pts: i32 },
    /// Account-wide secondary persistent timestamp.
    ///
    /// This includes only certain bot updates and secret one-to-one chats.
    Secondary { qts: i32 },
    /// Channel-specific persistent timestamp.
    ///
    /// This includes "megagroup", "broadcast" and "supergroup" channels.
    Channel { channel_id: i64, pts: i32 },
}
