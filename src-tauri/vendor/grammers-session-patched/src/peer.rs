// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::fmt;

use grammers_tl_types as tl;

/// A compact peer identifier.
/// ```
/// use std::mem::size_of;
/// assert_eq!(size_of::<grammers_session::defs::PeerId>(), size_of::<i64>());
/// ```
/// The [`PeerInfo`] cached by the session for this `PeerId` may be retrieved via [`crate::Session::peer`].
///
/// The internal representation uses the Bot API Dialog ID format to
/// bit-pack both the peer's true identifier and type in a single integer.
///
/// Internally, arbitrary values outside the valid range of Bot API Dialog ID
/// may be used to represent special peer identifiers.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PeerId(i64);

/// Witness to the session's authority from Telegram to interact with a peer.
///
/// If Telegram deems the session to already have such authority, the session may
/// be allowed to present [`PeerAuth::default`] instead of this witness. This can
/// happen when the logged-in user is a bot account, or, for user accounts, when
/// the peer being interacted with is one of its contacts.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PeerAuth(i64);

/// Ocap-style reference to a peer object, a peer object capability, bundling the identity
/// of a peer (its [`PeerId`]) with authority over it (as [`PeerAuth`]), to allow fluent use.
///
/// This type implements conversion to [`tl::enums::InputPeer`] and derivatives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PeerRef {
    /// The peer identity.
    pub id: PeerId,
    /// The authority bound to both the sibling identity and the session of the logged-in user.
    pub auth: PeerAuth,
}

/// [`PeerId`]'s kind.
///
/// The `PeerId` bitpacks this information for size reasons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PeerKind {
    /// The peer identity belongs to a [`tl::enums::User`]. May also represent [`PeerKind::UserSelf`].
    User,
    /// The peer identity belongs to a user with its [`tl::types::User::is_self`] flag set to `true`.
    UserSelf,
    /// The peer identity belongs to a [`tl::types::Chat`] or one of its derivatives.
    Chat,
    /// The peer identity belongs to a [`tl::types::Channel`] or one of its derivatives.
    Channel,
}

/// An exploded peer reference along with any known useful information about the peer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerInfo {
    User {
        /// Bare user identifier.
        ///
        /// Despite being `i64`, Telegram only uses strictly positive values.
        id: i64,
        /// Non-ambient authority bound to both the user itself and the session.
        auth: Option<PeerAuth>,
        /// Whether this user represents a bot or not.
        bot: Option<bool>,
        /// Whether this user represents the logged-in user authorized by this session or not.
        is_self: Option<bool>,
    },
    Chat {
        /// Bare chat identifier.
        ///
        /// Note that the HTTP Bot API negates this identifier to signal that it is a chat,
        /// but the true value used by Telegram's API is always strictly-positive.
        id: i64,
    },
    Channel {
        /// Bare channel identifier.
        ///
        /// Note that the HTTP Bot API prefixes this identifier with `-100` to signal that it is a channel,
        /// but the true value used by Telegram's API is always strictly-positive.
        id: i64,
        /// Non-ambient authority bound to both the user itself and the session.
        auth: Option<PeerAuth>,
        /// Channel kind, useful to determine what the possible permissions on it are.
        kind: Option<ChannelKind>,
    },
}

/// Additional information about a [`PeerInfo::Channel`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelKind {
    /// Value used for a channel with its [`tl::types::Channel::megagroup`] flag set to `true`.
    Megagroup,
    /// Value used for a channel with its [`tl::types::Channel::broadcast`] flag set to `true`.
    Broadcast,
    /// Value used for a channel with its [`tl::types::Channel::gigagroup`] flag set to `true`.
    Gigagroup,
}

/// Sentinel value used to represent the self-user when its true `PeerId` is unknown.
///
/// Per https://core.telegram.org/api/bots/ids:
/// > a bot API dialog ID ranges from -4000000000000 to 1099511627775
///
/// This value is not intended to be visible or persisted, so it can be changed as needed in the future.
const SELF_USER_ID: PeerId = PeerId(1 << 40);

/// Sentinel value used to represent empty chats.
///
/// Per https://core.telegram.org/api/bots/ids:
/// > \[…] transformed range for bot API chat dialog IDs is -999999999999 to -1 inclusively
/// >
/// > \[…] transformed range for bot API channel dialog IDs is -1997852516352 to -1000000000001 inclusively
///
/// `chat_id` parameters are in Telegram's API use the bare identifier, so there's no
/// empty constructor, but it can be mimicked by picking the value in the correct range hole.
/// This value is closer to "channel with ID 0" than "chat with ID 0", but there's no distinct
/// `-0` integer, and channels have a proper constructor for empty already
const EMPTY_CHAT_ID: i64 = -1000000000000;

impl PeerId {
    /// Creates a peer identity for the currently-logged-in user or bot account.
    ///
    /// Internally, this will use a special sentinel value outside of any valid Bot API Dialog ID range.
    pub fn self_user() -> Self {
        SELF_USER_ID
    }

    /// Creates a peer identity for a user or bot account.
    pub fn user(id: i64) -> Self {
        // https://core.telegram.org/api/bots/ids#user-ids
        if !(1 <= id && id <= 0xffffffffff) {
            panic!("user ID out of range");
        }

        Self(id)
    }

    /// Creates a peer identity for a small group chat.
    pub fn chat(id: i64) -> Self {
        // https://core.telegram.org/api/bots/ids#chat-ids
        if !(1 <= id && id <= 999999999999) {
            panic!("chat ID out of range");
        }

        Self(-id)
    }

    /// Creates a peer identity for a broadcast channel, megagroup, gigagroup or monoforum.
    pub fn channel(id: i64) -> Self {
        // https://core.telegram.org/api/bots/ids#supergroup-channel-ids and #monoforum-ids
        if !((1 <= id && id <= 997852516352) || (1002147483649 <= id && id <= 3000000000000)) {
            panic!("channel ID out of range");
        }

        Self(-(1000000000000 + id))
    }

    /// Peer kind.
    pub fn kind(self) -> PeerKind {
        if 1 <= self.0 && self.0 <= 0xffffffffff {
            PeerKind::User
        } else if self.0 == SELF_USER_ID.0 {
            PeerKind::UserSelf
        } else if -999999999999 <= self.0 && self.0 <= -1 {
            PeerKind::Chat
        } else if -1997852516352 <= self.0 && self.0 <= -1000000000001
            || (-4000000000000 <= self.0 && self.0 <= -2002147483649)
        {
            PeerKind::Channel
        } else {
            unreachable!()
        }
    }

    /// Returns the identity using the Bot API Dialog ID format.
    ///
    /// Will return an arbitrary value if [`Self::kind`] is [`PeerKind::UserSelf`].
    /// This value should not be relied on and may change between releases.
    pub fn bot_api_dialog_id(&self) -> i64 {
        self.0
    }

    /// Unpacked peer identifier. Panics if [`Self::kind`] is [`PeerKind::UserSelf`].
    pub fn bare_id(&self) -> i64 {
        match self.kind() {
            PeerKind::User => self.0,
            PeerKind::UserSelf => panic!("self-user ID not known"),
            PeerKind::Chat => -self.0,
            PeerKind::Channel => -self.0 - 1000000000000,
        }
    }
}

impl PeerAuth {
    /// Construct a new peer authentication using Telegram's `access_hash` value.
    pub fn from_hash(access_hash: i64) -> Self {
        PeerAuth(access_hash)
    }

    /// Grants access to the internal access hash.
    pub fn hash(&self) -> i64 {
        self.0
    }
}

impl Default for PeerAuth {
    /// Returns the ambient authority to authorize peers only when Telegram considers it valid.
    ///
    /// The internal representation uses `0` to signal the ambient authority,
    /// although this might happen to be the actual witness used by some peers.
    fn default() -> Self {
        Self(0)
    }
}

impl PeerInfo {
    /// Returns the `PeerId` represented by this info.
    ///
    /// The returned [`PeerId::kind()`] will never be [`PeerKind::UserSelf`].
    pub fn id(&self) -> PeerId {
        match self {
            PeerInfo::User { id, .. } => PeerId::user(*id),
            PeerInfo::Chat { id } => PeerId::chat(*id),
            PeerInfo::Channel { id, .. } => PeerId::channel(*id),
        }
    }

    /// Returns the `PeerAuth` stored in this info, or [`PeerAuth::default()`] if that info is not known.
    pub fn auth(&self) -> PeerAuth {
        match self {
            PeerInfo::User { auth, .. } => auth.unwrap_or_default(),
            PeerInfo::Chat { .. } => PeerAuth::default(),
            PeerInfo::Channel { auth, .. } => auth.unwrap_or_default(),
        }
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.bot_api_dialog_id().fmt(f)
    }
}

impl From<PeerInfo> for PeerRef {
    fn from(peer: PeerInfo) -> Self {
        PeerRef {
            id: peer.id(),
            auth: peer.auth(),
        }
    }
}

impl From<tl::enums::Peer> for PeerId {
    fn from(peer: tl::enums::Peer) -> Self {
        match peer {
            tl::enums::Peer::User(user) => PeerId::from(user),
            tl::enums::Peer::Chat(chat) => PeerId::from(chat),
            tl::enums::Peer::Channel(channel) => PeerId::from(channel),
        }
    }
}

impl From<tl::types::PeerUser> for PeerId {
    fn from(user: tl::types::PeerUser) -> Self {
        PeerId::user(user.user_id)
    }
}

impl From<tl::types::PeerChat> for PeerId {
    fn from(chat: tl::types::PeerChat) -> Self {
        PeerId::chat(chat.chat_id)
    }
}

impl From<tl::types::PeerChannel> for PeerId {
    fn from(channel: tl::types::PeerChannel) -> Self {
        PeerId::channel(channel.channel_id)
    }
}

impl From<tl::enums::InputPeer> for PeerRef {
    fn from(peer: tl::enums::InputPeer) -> Self {
        match peer {
            tl::enums::InputPeer::Empty => {
                panic!("InputPeer::Empty cannot be converted to any Peer");
            }
            tl::enums::InputPeer::PeerSelf => PeerRef {
                id: SELF_USER_ID,
                auth: PeerAuth::default(),
            },
            tl::enums::InputPeer::User(user) => PeerRef::from(user),
            tl::enums::InputPeer::Chat(chat) => PeerRef::from(chat),
            tl::enums::InputPeer::Channel(channel) => PeerRef::from(channel),
            tl::enums::InputPeer::UserFromMessage(user) => PeerRef::from(*user),
            tl::enums::InputPeer::ChannelFromMessage(channel) => PeerRef::from(*channel),
        }
    }
}

impl From<tl::types::InputPeerSelf> for PeerRef {
    fn from(_: tl::types::InputPeerSelf) -> Self {
        PeerRef {
            id: SELF_USER_ID,
            auth: PeerAuth::default(),
        }
    }
}

impl From<tl::types::InputPeerUser> for PeerRef {
    fn from(user: tl::types::InputPeerUser) -> Self {
        PeerRef {
            id: PeerId::user(user.user_id),
            auth: PeerAuth::from_hash(user.access_hash),
        }
    }
}

impl From<tl::types::InputPeerChat> for PeerRef {
    fn from(chat: tl::types::InputPeerChat) -> Self {
        PeerRef {
            id: PeerId::chat(chat.chat_id),
            auth: PeerAuth::default(),
        }
    }
}

impl From<tl::types::InputPeerChannel> for PeerRef {
    fn from(channel: tl::types::InputPeerChannel) -> Self {
        PeerRef {
            id: PeerId::channel(channel.channel_id),
            auth: PeerAuth::from_hash(channel.access_hash),
        }
    }
}

impl From<tl::types::InputPeerUserFromMessage> for PeerRef {
    fn from(user: tl::types::InputPeerUserFromMessage) -> Self {
        // Not currently willing to make PeerRef significantly larger to accomodate for this uncommon type.
        PeerRef {
            id: PeerId::user(user.user_id),
            auth: PeerAuth::default(),
        }
    }
}

impl From<tl::types::InputPeerChannelFromMessage> for PeerRef {
    fn from(channel: tl::types::InputPeerChannelFromMessage) -> Self {
        // Not currently willing to make PeerRef significantly larger to accomodate for this uncommon type.
        PeerRef {
            id: PeerId::channel(channel.channel_id),
            auth: PeerAuth::default(),
        }
    }
}

impl From<tl::enums::User> for PeerRef {
    fn from(user: tl::enums::User) -> Self {
        match user {
            grammers_tl_types::enums::User::Empty(user) => PeerRef::from(user),
            grammers_tl_types::enums::User::User(user) => PeerRef::from(user),
        }
    }
}

impl From<tl::types::UserEmpty> for PeerRef {
    fn from(user: tl::types::UserEmpty) -> Self {
        PeerRef {
            id: PeerId::user(user.id),
            auth: PeerAuth::default(),
        }
    }
}

impl From<tl::types::User> for PeerRef {
    fn from(user: tl::types::User) -> Self {
        PeerRef {
            id: if user.is_self {
                PeerId::self_user()
            } else {
                PeerId::user(user.id)
            },
            auth: user
                .access_hash
                .map(PeerAuth::from_hash)
                .unwrap_or(PeerAuth::default()),
        }
    }
}

impl From<tl::enums::Chat> for PeerRef {
    fn from(chat: tl::enums::Chat) -> Self {
        match chat {
            grammers_tl_types::enums::Chat::Empty(chat) => PeerRef::from(chat),
            grammers_tl_types::enums::Chat::Chat(chat) => PeerRef::from(chat),
            grammers_tl_types::enums::Chat::Forbidden(chat) => PeerRef::from(chat),
            grammers_tl_types::enums::Chat::Channel(channel) => PeerRef::from(channel),
            grammers_tl_types::enums::Chat::ChannelForbidden(channel) => PeerRef::from(channel),
        }
    }
}

impl From<tl::types::ChatEmpty> for PeerRef {
    fn from(chat: tl::types::ChatEmpty) -> Self {
        PeerRef {
            id: PeerId::chat(chat.id),
            auth: PeerAuth::default(),
        }
    }
}

impl From<tl::types::Chat> for PeerRef {
    fn from(chat: tl::types::Chat) -> Self {
        PeerRef {
            id: PeerId::chat(chat.id),
            auth: PeerAuth::default(),
        }
    }
}

impl From<tl::types::ChatForbidden> for PeerRef {
    fn from(chat: tl::types::ChatForbidden) -> Self {
        PeerRef {
            id: PeerId::chat(chat.id),
            auth: PeerAuth::default(),
        }
    }
}

impl From<tl::types::Channel> for PeerRef {
    fn from(channel: tl::types::Channel) -> Self {
        PeerRef {
            id: PeerId::channel(channel.id),
            auth: channel
                .access_hash
                .map(PeerAuth::from_hash)
                .unwrap_or(PeerAuth::default()),
        }
    }
}

impl From<tl::types::ChannelForbidden> for PeerRef {
    fn from(channel: tl::types::ChannelForbidden) -> Self {
        PeerRef {
            id: PeerId::channel(channel.id),
            auth: PeerAuth::from_hash(channel.access_hash),
        }
    }
}

impl From<PeerId> for tl::enums::Peer {
    fn from(peer: PeerId) -> Self {
        match peer.kind() {
            PeerKind::User => tl::enums::Peer::User(tl::types::PeerUser {
                user_id: peer.bare_id(),
            }),
            PeerKind::UserSelf => panic!("self-user ID not known"),
            PeerKind::Chat => tl::enums::Peer::Chat(tl::types::PeerChat {
                chat_id: peer.bare_id(),
            }),
            PeerKind::Channel => tl::enums::Peer::Channel(tl::types::PeerChannel {
                channel_id: peer.bare_id(),
            }),
        }
    }
}

impl From<PeerRef> for tl::enums::InputPeer {
    fn from(peer: PeerRef) -> Self {
        match peer.id.kind() {
            PeerKind::User => tl::enums::InputPeer::User(tl::types::InputPeerUser {
                user_id: peer.id.bare_id(),
                access_hash: peer.auth.hash(),
            }),
            PeerKind::UserSelf => tl::enums::InputPeer::PeerSelf,
            PeerKind::Chat => tl::enums::InputPeer::Chat(tl::types::InputPeerChat {
                chat_id: peer.id.bare_id(),
            }),
            PeerKind::Channel => tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                channel_id: peer.id.bare_id(),
                access_hash: peer.auth.hash(),
            }),
        }
    }
}

impl From<PeerRef> for tl::enums::InputUser {
    fn from(peer: PeerRef) -> Self {
        match peer.id.kind() {
            PeerKind::User => tl::enums::InputUser::User(tl::types::InputUser {
                user_id: peer.id.bare_id(),
                access_hash: peer.auth.hash(),
            }),
            PeerKind::UserSelf => tl::enums::InputUser::UserSelf,
            PeerKind::Chat => tl::enums::InputUser::Empty,
            PeerKind::Channel => tl::enums::InputUser::Empty,
        }
    }
}

impl From<PeerRef> for i64 {
    fn from(peer: PeerRef) -> Self {
        match peer.id.kind() {
            PeerKind::User => EMPTY_CHAT_ID,
            PeerKind::UserSelf => EMPTY_CHAT_ID,
            PeerKind::Chat => peer.id.bare_id(),
            PeerKind::Channel => EMPTY_CHAT_ID,
        }
    }
}

impl From<PeerRef> for tl::enums::InputChannel {
    fn from(peer: PeerRef) -> Self {
        match peer.id.kind() {
            PeerKind::User => tl::enums::InputChannel::Empty,
            PeerKind::UserSelf => tl::enums::InputChannel::Empty,
            PeerKind::Chat => tl::enums::InputChannel::Empty,
            PeerKind::Channel => tl::enums::InputChannel::Channel(tl::types::InputChannel {
                channel_id: peer.id.bare_id(),
                access_hash: peer.auth.hash(),
            }),
        }
    }
}
