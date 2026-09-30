// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
use super::defs::{Gap, Key, NO_PTS, NO_SEQ, PtsInfo, UpdatesLike};
use grammers_tl_types as tl;
use log::info;

// > The `updateShortMessage`, `updateShortSentMessage` and `updateShortChatMessage` constructors
// > [...] should be transformed to `updateShort` upon receiving.
//
// We don't use `updateShort` because it's fairly limitting. `updatesCombined` is big enough to
// contain all updates inside, and sticking to a single type (and not enum) simplifies things.
//
// This module's job is just converting the various updates types into `updatesCombined`.
// It also converts updates into their corresponding `PtsInfo`.

fn updates(updates: tl::types::Updates) -> tl::types::UpdatesCombined {
    tl::types::UpdatesCombined {
        updates: updates.updates,
        users: updates.users,
        chats: updates.chats,
        date: updates.date,
        seq_start: updates.seq,
        seq: updates.seq,
    }
}

fn update_short(short: tl::types::UpdateShort) -> tl::types::UpdatesCombined {
    tl::types::UpdatesCombined {
        updates: vec![short.update],
        users: Vec::new(),
        chats: Vec::new(),
        date: short.date,
        seq_start: NO_SEQ,
        seq: NO_SEQ,
    }
}

fn update_short_message(short: tl::types::UpdateShortMessage) -> tl::types::UpdatesCombined {
    update_short(tl::types::UpdateShort {
        update: tl::types::UpdateNewMessage {
            message: tl::types::Message {
                out: short.out,
                mentioned: short.mentioned,
                media_unread: short.media_unread,
                silent: short.silent,
                post: false,
                from_scheduled: false,
                legacy: false,
                edit_hide: false,
                pinned: false,
                noforwards: false,
                invert_media: false,
                video_processing_pending: false,
                paid_suggested_post_stars: false,
                reactions: None,
                id: short.id,
                from_id: None,
                from_boosts_applied: None,
                peer_id: tl::types::PeerChat {
                    chat_id: short.user_id,
                }
                .into(),
                saved_peer_id: None,
                fwd_from: short.fwd_from,
                via_bot_id: short.via_bot_id,
                reply_to: short.reply_to,
                date: short.date,
                message: short.message,
                media: None,
                reply_markup: None,
                entities: short.entities,
                views: None,
                forwards: None,
                replies: None,
                edit_date: None,
                post_author: None,
                grouped_id: None,
                restriction_reason: None,
                ttl_period: short.ttl_period,
                quick_reply_shortcut_id: None,
                offline: false,
                via_business_bot_id: None,
                effect: None,
                factcheck: None,
                report_delivery_until_date: None,
                paid_message_stars: None,
                paid_suggested_post_ton: false,
                suggested_post: None,
            }
            .into(),
            pts: short.pts,
            pts_count: short.pts_count,
        }
        .into(),
        date: short.date,
    })
}

fn update_short_chat_message(
    short: tl::types::UpdateShortChatMessage,
) -> tl::types::UpdatesCombined {
    update_short(tl::types::UpdateShort {
        update: tl::types::UpdateNewMessage {
            message: tl::types::Message {
                out: short.out,
                mentioned: short.mentioned,
                media_unread: short.media_unread,
                silent: short.silent,
                post: false,
                from_scheduled: false,
                legacy: false,
                edit_hide: false,
                pinned: false,
                noforwards: false,
                invert_media: false,
                video_processing_pending: false,
                paid_suggested_post_stars: false,
                reactions: None,
                id: short.id,
                from_id: Some(
                    tl::types::PeerUser {
                        user_id: short.from_id,
                    }
                    .into(),
                ),
                from_boosts_applied: None,
                peer_id: tl::types::PeerChat {
                    chat_id: short.chat_id,
                }
                .into(),
                saved_peer_id: None,
                fwd_from: short.fwd_from,
                via_bot_id: short.via_bot_id,
                reply_to: short.reply_to,
                date: short.date,
                message: short.message,
                media: None,
                reply_markup: None,
                entities: short.entities,
                views: None,
                forwards: None,
                replies: None,
                edit_date: None,
                post_author: None,
                grouped_id: None,
                restriction_reason: None,
                ttl_period: short.ttl_period,
                quick_reply_shortcut_id: None,
                offline: false,
                via_business_bot_id: None,
                effect: None,
                factcheck: None,
                report_delivery_until_date: None,
                paid_message_stars: None,
                paid_suggested_post_ton: false,
                suggested_post: None,
            }
            .into(),
            pts: short.pts,
            pts_count: short.pts_count,
        }
        .into(),
        date: short.date,
    })
}

fn update_short_sent_message(
    short: tl::types::UpdateShortSentMessage,
) -> tl::types::UpdatesCombined {
    update_short(tl::types::UpdateShort {
        update: tl::types::UpdateNewMessage {
            message: tl::types::MessageEmpty {
                id: short.id,
                peer_id: None,
            }
            .into(),
            pts: short.pts,
            pts_count: short.pts_count,
        }
        .into(),
        date: short.date,
    })
}

fn adapt_updates(updates: tl::enums::Updates) -> Result<tl::types::UpdatesCombined, Gap> {
    Ok(match updates {
        // > `updatesTooLong` indicates that there are too many events pending to be pushed
        // > to the client, so one needs to fetch them manually.
        tl::enums::Updates::TooLong => {
            info!("received updatesTooLong, treating as gap");
            return Err(Gap);
        }
        // > `updateShortMessage`, `updateShortSentMessage` and `updateShortChatMessage` [...]
        // > should be transformed to `updateShort` upon receiving.
        tl::enums::Updates::UpdateShortMessage(short) => {
            // > Incomplete update: the client is missing data about a chat/user from one of
            // > the shortened constructors, such as `updateShortChatMessage`, etc.
            //
            // The check for missing hashes is done elsewhere to avoid doing the same work twice.
            // This "only" needs to be done for "short messages", to get the private chat (user)
            // where the message occured. Anywhere else, Telegram should send information
            // about the chat so that [min constructors][0] can be used.
            //
            // [0]: https://core.telegram.org/api/min
            update_short_message(short)
        }
        tl::enums::Updates::UpdateShortChatMessage(short) => {
            // No need to check for chats here. Small group chats do not require an access
            // hash, and min constructors can be used to access the user.
            update_short_chat_message(short)
        }
        // > `updateShort` […] have lower priority and are broadcast to a large number of users.
        //
        // There *shouldn't* be updates mentioning peers we're unaware of here.
        //
        // If later it turns out these can happen, the code will need to be updated to
        // consider chats missing here a gap as well.
        tl::enums::Updates::UpdateShort(short) => update_short(short),
        // > [the] `seq` attribute, which indicates the remote `Updates` state after the
        // > generation of the `Updates`, and `seq_start` indicates the remote `Updates` state
        // > after the first of the `Updates` in the packet is generated
        tl::enums::Updates::Combined(combined) => combined,
        // > [the] `seq_start` attribute is omitted, because it is assumed that it is always
        // > equal to `seq`.
        tl::enums::Updates::Updates(updates) => self::updates(updates),
        // Even though we lack fields like the message text, it still needs to be handled, so
        // that the `pts` can be kept consistent.
        tl::enums::Updates::UpdateShortSentMessage(short) => update_short_sent_message(short),
    })
}

pub(super) fn adapt(updates: UpdatesLike) -> Result<tl::types::UpdatesCombined, Gap> {
    match updates {
        UpdatesLike::Updates(updates) => adapt_updates(updates),
        UpdatesLike::ShortSentMessage { request, update } => {
            Ok(update_short(tl::types::UpdateShort {
                update: tl::types::UpdateNewMessage {
                    message: tl::types::Message {
                        out: update.out,
                        mentioned: false,
                        media_unread: false,
                        silent: request.silent,
                        post: false,
                        from_scheduled: false,
                        legacy: false,
                        edit_hide: false,
                        pinned: false,
                        noforwards: request.noforwards,
                        invert_media: request.invert_media,
                        offline: false,
                        video_processing_pending: false,
                        paid_suggested_post_stars: false,
                        paid_suggested_post_ton: false,
                        id: update.id,
                        from_id: request.send_as.as_ref().map(peer_from_input_peer),
                        from_boosts_applied: None,
                        peer_id: peer_from_input_peer(&request.peer),
                        saved_peer_id: None,
                        fwd_from: None,
                        via_bot_id: None,
                        via_business_bot_id: None,
                        reply_to: request
                            .reply_to
                            .map(|r| match r {
                                tl::enums::InputReplyTo::Message(i) => {
                                    Some(tl::enums::MessageReplyHeader::Header(
                                        tl::types::MessageReplyHeader {
                                            reply_to_scheduled: false,
                                            forum_topic: false,
                                            quote: i.quote_offset.is_some(),
                                            reply_to_msg_id: Some(i.reply_to_msg_id),
                                            reply_to_peer_id: i
                                                .reply_to_peer_id
                                                .as_ref()
                                                .map(peer_from_input_peer),
                                            reply_from: None,
                                            reply_media: None,
                                            reply_to_top_id: i.top_msg_id,
                                            quote_text: i.quote_text,
                                            quote_entities: i.quote_entities,
                                            quote_offset: i.quote_offset,
                                            todo_item_id: None,
                                        },
                                    ))
                                }
                                tl::enums::InputReplyTo::Story(i) => {
                                    Some(tl::enums::MessageReplyHeader::MessageReplyStoryHeader(
                                        tl::types::MessageReplyStoryHeader {
                                            peer: peer_from_input_peer(&i.peer),
                                            story_id: i.story_id,
                                        },
                                    ))
                                }
                                tl::enums::InputReplyTo::MonoForum(_) => None,
                            })
                            .flatten(),
                        date: update.date,
                        message: request.message,
                        media: update.media,
                        reply_markup: request.reply_markup,
                        entities: update.entities.or(request.entities),
                        views: None,
                        forwards: None,
                        replies: None,
                        edit_date: None,
                        post_author: None,
                        grouped_id: None,
                        reactions: None,
                        restriction_reason: None,
                        ttl_period: update.ttl_period,
                        quick_reply_shortcut_id: request.quick_reply_shortcut.and_then(
                            |q| match q {
                                tl::enums::InputQuickReplyShortcut::Shortcut(_) => None,
                                tl::enums::InputQuickReplyShortcut::Id(i) => Some(i.shortcut_id),
                            },
                        ),
                        effect: request.effect,
                        factcheck: None,
                        report_delivery_until_date: None,
                        paid_message_stars: None,
                        suggested_post: None,
                    }
                    .into(),
                    pts: update.pts,
                    pts_count: update.pts_count,
                }
                .into(),
                date: update.date,
            }))
        }
        // For simplicity, instead of introducing an extra enum, reuse a closely-related update type.
        UpdatesLike::AffectedMessages(affected) => Ok(update_short(tl::types::UpdateShort {
            update: tl::types::UpdateDeleteMessages {
                messages: Vec::new(),
                pts: affected.pts,
                pts_count: affected.pts_count,
            }
            .into(),
            date: 0,
        })),
        UpdatesLike::InvitedUsers(invited) => adapt_updates(invited.updates),
    }
}

pub(super) fn adapt_channel_difference(
    difference: tl::enums::updates::ChannelDifference,
) -> tl::types::updates::ChannelDifference {
    match difference {
        tl::enums::updates::ChannelDifference::Empty(difference) => {
            tl::types::updates::ChannelDifference {
                r#final: difference.r#final,
                pts: difference.pts,
                timeout: difference.timeout,
                new_messages: Vec::new(),
                other_updates: Vec::new(),
                chats: Vec::new(),
                users: Vec::new(),
            }
        }
        tl::enums::updates::ChannelDifference::TooLong(difference) => {
            tl::types::updates::ChannelDifference {
                r#final: difference.r#final,
                pts: match difference.dialog {
                    tl::enums::Dialog::Dialog(d) => d
                        .pts
                        .expect("channelDifferenceTooLong dialog did not actually contain a pts"),
                    tl::enums::Dialog::Folder(_) => {
                        panic!("received a folder on channelDifferenceTooLong")
                    }
                },
                timeout: difference.timeout,
                new_messages: Vec::new(),
                other_updates: Vec::new(),
                chats: difference.chats,
                users: difference.users,
            }
        }
        tl::enums::updates::ChannelDifference::Difference(difference) => difference,
    }
}

fn peer_from_input_peer(input_peer: &tl::enums::InputPeer) -> tl::enums::Peer {
    match input_peer {
        grammers_tl_types::enums::InputPeer::Empty => tl::types::PeerUser { user_id: 0 }.into(),
        grammers_tl_types::enums::InputPeer::PeerSelf => tl::types::PeerUser { user_id: 0 }.into(), // TODO can get self from client
        grammers_tl_types::enums::InputPeer::Chat(chat) => tl::types::PeerChat {
            chat_id: chat.chat_id,
        }
        .into(),
        grammers_tl_types::enums::InputPeer::User(user) => tl::types::PeerUser {
            user_id: user.user_id,
        }
        .into(),
        grammers_tl_types::enums::InputPeer::Channel(channel) => tl::types::PeerChannel {
            channel_id: channel.channel_id,
        }
        .into(),
        grammers_tl_types::enums::InputPeer::UserFromMessage(user) => tl::types::PeerUser {
            user_id: user.user_id,
        }
        .into(),
        grammers_tl_types::enums::InputPeer::ChannelFromMessage(channel) => {
            tl::types::PeerChannel {
                channel_id: channel.channel_id,
            }
            .into()
        }
    }
}

fn message_peer(message: &tl::enums::Message) -> Option<tl::enums::Peer> {
    match message {
        tl::enums::Message::Empty(_) => None,
        tl::enums::Message::Message(m) => Some(m.peer_id.clone()),
        tl::enums::Message::Service(m) => Some(m.peer_id.clone()),
    }
}

fn message_channel_id(message: &tl::enums::Message) -> Option<i64> {
    match message {
        tl::enums::Message::Empty(_) => None,
        tl::enums::Message::Message(m) => match &m.peer_id {
            tl::enums::Peer::Channel(c) => Some(c.channel_id),
            _ => None,
        },
        tl::enums::Message::Service(m) => match &m.peer_id {
            tl::enums::Peer::Channel(c) => Some(c.channel_id),
            _ => None,
        },
    }
}

impl PtsInfo {
    pub(super) fn from_update(update: &tl::enums::Update) -> Option<Self> {
        use tl::enums::Update::*;
        match update {
            NewMessage(u) => {
                assert!(!matches!(
                    message_peer(&u.message),
                    Some(tl::enums::Peer::Channel(_))
                ));
                Some(Self {
                    key: Key::Common,
                    pts: u.pts,
                    count: u.pts_count,
                })
            }
            MessageId(_) => None,
            DeleteMessages(u) => Some(Self {
                key: Key::Common,
                pts: u.pts,
                count: u.pts_count,
            }),
            UserTyping(_) => None,
            ChatUserTyping(_) => None,
            ChatParticipants(_) => None,
            UserStatus(_) => None,
            UserName(_) => None,
            NewAuthorization(_) => None,
            NewEncryptedMessage(u) => Some(Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1,
            }),
            EncryptedChatTyping(_) => None,
            Encryption(_) => None,
            EncryptedMessagesRead(_) => None,
            ChatParticipantAdd(_) => None,
            ChatParticipantDelete(_) => None,
            DcOptions(_) => None,
            NotifySettings(_) => None,
            ServiceNotification(_) => None,
            Privacy(_) => None,
            UserPhone(_) => None,
            ReadHistoryInbox(u) => {
                assert!(!matches!(u.peer, tl::enums::Peer::Channel(_)));
                Some(Self {
                    key: Key::Common,
                    pts: u.pts,
                    count: u.pts_count,
                })
            }
            ReadHistoryOutbox(u) => {
                assert!(!matches!(u.peer, tl::enums::Peer::Channel(_)));
                Some(Self {
                    key: Key::Common,
                    pts: u.pts,
                    count: u.pts_count,
                })
            }
            WebPage(u) => Some(Self {
                key: Key::Common,
                pts: u.pts,
                count: u.pts_count,
            }),
            ReadMessagesContents(u) => Some(Self {
                key: Key::Common,
                pts: u.pts,
                count: u.pts_count,
            }),
            ChannelTooLong(u) => u.pts.map(|pts| Self {
                key: Key::Channel(u.channel_id),
                pts,
                count: 0,
            }),
            Channel(_) => None,
            // Telegram actually sends `updateNewChannelMessage(messageEmpty(…))`, and because
            // there's no way to tell which channel ID this `pts` belongs to, the best we can
            // do is ignore it.
            //
            // Future messages should trigger a gap that we need to recover from.
            NewChannelMessage(u) => message_channel_id(&u.message).map(|channel_id| Self {
                key: Key::Channel(channel_id),
                pts: u.pts,
                count: u.pts_count,
            }),
            ReadChannelInbox(u) => Some(Self {
                key: Key::Channel(u.channel_id),
                pts: u.pts,
                count: 0,
            }),
            DeleteChannelMessages(u) => Some(Self {
                key: Key::Channel(u.channel_id),
                pts: u.pts,
                count: u.pts_count,
            }),
            ChannelMessageViews(_) => None,
            ChatParticipantAdmin(_) => None,
            NewStickerSet(_) => None,
            StickerSetsOrder(_) => None,
            StickerSets(_) => None,
            SavedGifs => None,
            BotInlineQuery(_) => None,
            BotInlineSend(_) => None,
            EditChannelMessage(u) => message_channel_id(&u.message).map(|channel_id| Self {
                key: Key::Channel(channel_id),
                pts: u.pts,
                count: u.pts_count,
            }),
            BotCallbackQuery(_) => None,
            EditMessage(u) => {
                assert!(!matches!(
                    message_peer(&u.message),
                    Some(tl::enums::Peer::Channel(_))
                ));
                Some(Self {
                    key: Key::Common,
                    pts: u.pts,
                    count: u.pts_count,
                })
            }
            InlineBotCallbackQuery(_) => None,
            ReadChannelOutbox(_) => None,
            DraftMessage(_) => None,
            ReadFeaturedStickers => None,
            RecentStickers => None,
            Config => None,
            PtsChanged => None,
            ChannelWebPage(u) => Some(Self {
                key: Key::Channel(u.channel_id),
                pts: u.pts,
                count: u.pts_count,
            }),
            DialogPinned(_) => None,
            PinnedDialogs(_) => None,
            BotWebhookJson(_) => None,
            BotWebhookJsonquery(_) => None,
            BotShippingQuery(_) => None,
            BotPrecheckoutQuery(_) => None,
            PhoneCall(_) => None,
            LangPackTooLong(_) => None,
            LangPack(_) => None,
            FavedStickers => None,
            ChannelReadMessagesContents(_) => None,
            ContactsReset => None,
            ChannelAvailableMessages(_) => None,
            DialogUnreadMark(_) => None,
            MessagePoll(_) => None,
            ChatDefaultBannedRights(_) => None,
            FolderPeers(u) => Some(Self {
                key: Key::Common,
                pts: u.pts,
                count: u.pts_count,
            }),
            PeerSettings(_) => None,
            PeerLocated(_) => None,
            NewScheduledMessage(_) => None,
            DeleteScheduledMessages(_) => None,
            Theme(_) => None,
            GeoLiveViewed(_) => None,
            LoginToken => None,
            MessagePollVote(_) => None,
            DialogFilter(_) => None,
            DialogFilterOrder(_) => None,
            DialogFilters => None,
            PhoneCallSignalingData(_) => None,
            ChannelMessageForwards(_) => None,
            ReadChannelDiscussionInbox(_) => None,
            ReadChannelDiscussionOutbox(_) => None,
            PeerBlocked(_) => None,
            ChannelUserTyping(_) => None,
            PinnedMessages(u) => {
                assert!(!matches!(u.peer, tl::enums::Peer::Channel(_)));
                Some(Self {
                    key: Key::Common,
                    pts: u.pts,
                    count: u.pts_count,
                })
            }
            PinnedChannelMessages(u) => Some(Self {
                key: Key::Channel(u.channel_id),
                pts: u.pts,
                count: u.pts_count,
            }),
            Chat(_) => None,
            GroupCallParticipants(_) => None,
            GroupCall(_) => None,
            PeerHistoryTtl(_) => None,
            ChatParticipant(u) => Some(Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1,
            }),
            ChannelParticipant(u) => Some(Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1,
            }),
            BotStopped(u) => Some(Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1,
            }),
            GroupCallConnection(_) => None,
            BotCommands(_) => None,
            PendingJoinRequests(_) => None,
            BotChatInviteRequester(u) => Some(Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1,
            }),
            MessageReactions(_) => None,
            AttachMenuBots => None,
            WebViewResultSent(_) => None,
            BotMenuButton(_) => None,
            SavedRingtones => None,
            TranscribedAudio(_) => None,
            ReadFeaturedEmojiStickers => None,
            UserEmojiStatus(_) => None,
            RecentEmojiStatuses => None,
            RecentReactions => None,
            MoveStickerSetToTop(_) => None,
            MessageExtendedMedia(_) => None,
            User(_) => None,
            AutoSaveSettings => None,
            Story(_) => None,
            NewStoryReaction(_) => None,
            ReadStories(_) => None,
            StoryId(_) => None,
            StoriesStealthMode(_) => None,
            SentStoryReaction(_) => None,
            BotChatBoost(u) => Some(Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1,
            }),
            ChannelViewForumAsMessages(_) => None,
            PeerWallpaper(_) => None,
            BotMessageReaction(u) => Some(Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1,
            }),
            BotMessageReactions(u) => Some(Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1,
            }),
            SavedDialogPinned(_) => None,
            PinnedSavedDialogs(_) => None,
            SavedReactionTags => None,
            SmsJob(_) => None,
            QuickReplies(_) => None,
            NewQuickReply(_) => None,
            DeleteQuickReply(_) => None,
            QuickReplyMessage(_) => None,
            DeleteQuickReplyMessages(_) => None,
            BotBusinessConnect(_) => None,
            BotNewBusinessMessage(u) => Some(Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1,
            }),
            BotEditBusinessMessage(u) => Some(Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1,
            }),
            BotDeleteBusinessMessage(u) => Some(Self {
                key: Key::Secondary,
                pts: u.qts,
                count: u.messages.len() as i32,
            }),
            StarsBalance(_) => None,
            BusinessBotCallbackQuery(_) => None,
            StarsRevenueStatus(_) => None,
            BotPurchasedPaidMedia(u) => Some(Self {
                key: Key::Secondary,
                pts: u.qts,
                count: 1, // TODO unsure if 1
            }),
            PaidReactionPrivacy(_) => None,
            _ => None,
        }
        .filter(|info| info.pts != NO_PTS)
    }
}
