// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! This module deals with correct handling of updates, including gaps, and knowing when the code
//! should "get difference" (the set of updates that the client should know by now minus the set
//! of updates that it actually knows).
//!
//! Each chat has its own [`Entry`] in the [`MessageBoxes`] (this `struct` is the "entry point").
//! At any given time, the message box may be either getting difference for them (entry is in
//! [`MessageBoxes::getting_diff_for`]) or not. If not getting difference, a possible gap may be
//! found for the updates (entry is in [`MessageBoxes::possible_gaps`]). Otherwise, the entry is
//! on its happy path.
//!
//! Gaps are cleared when they are either resolved on their own (by waiting for a short time)
//! or because we got the difference for the corresponding entry.
//!
//! While there are entries for which their difference must be fetched,
//! [`MessageBoxes::check_deadlines`] will always return [`Instant::now`], since "now" is the time
//! to get the difference.
mod adaptor;
mod defs;
#[cfg(test)]
mod tests;

use crate::defs::{ChannelState, UpdatesState};
use defs::Key;
pub use defs::{Gap, MessageBox, MessageBoxes, State, UpdatesLike};
use defs::{LiveEntry, NO_DATE, NO_PTS, NO_SEQ, POSSIBLE_GAP_TIMEOUT, PossibleGap, PtsInfo};
use grammers_tl_types as tl;
use log::{debug, info, trace};
use std::cmp::Ordering;
use std::time::Duration;
#[cfg(not(test))]
use std::time::Instant;
#[cfg(test)]
use tests::Instant;

fn next_updates_deadline() -> Instant {
    Instant::now() + defs::NO_UPDATES_TIMEOUT
}

impl MessageBox {
    pub fn pts(&self) -> i32 {
        match self {
            MessageBox::Common { pts } => *pts,
            MessageBox::Secondary { qts } => *qts,
            MessageBox::Channel { pts, .. } => *pts,
        }
    }
}

impl From<PtsInfo> for MessageBox {
    fn from(value: PtsInfo) -> Self {
        match value.key {
            Key::Common => Self::Common { pts: value.pts },
            Key::Secondary => Self::Secondary { qts: value.pts },
            Key::Channel(channel_id) => Self::Channel {
                channel_id,
                pts: value.pts,
            },
        }
    }
}

impl LiveEntry {
    fn effective_deadline(&self) -> Instant {
        match &self.possible_gap {
            Some(gap) => gap.deadline.min(self.deadline),
            None => self.deadline,
        }
    }
}

#[allow(clippy::new_without_default)]
/// Creation, querying, and setting base state.
impl MessageBoxes {
    /// Create a new, empty [`MessageBoxes`].
    pub fn new() -> Self {
        trace!("created new message box with no previous state");
        Self {
            entries: Vec::new(),
            date: NO_DATE,
            seq: NO_SEQ,
            getting_diff_for: Vec::new(),
            possible_gaps: Vec::new(),
            next_deadline: next_updates_deadline(),
        }
    }

    /// Create a [`MessageBoxes`] from a previously known update state.
    pub fn load(state: UpdatesState) -> Self {
        trace!("created new message box with state: {:?}", state);
        let mut entries = Vec::with_capacity(2 + state.channels.len());
        let mut getting_diff_for = Vec::with_capacity(2 + state.channels.len());
        let possible_gaps = Vec::with_capacity(2 + state.channels.len());
        let deadline = next_updates_deadline();

        if state.pts != NO_PTS {
            entries.push(LiveEntry {
                key: Key::Common,
                pts: state.pts,
                deadline,
                possible_gap: None,
            });
        }
        if state.qts != NO_PTS {
            entries.push(LiveEntry {
                key: Key::Secondary,
                pts: state.qts,
                deadline,
                possible_gap: None,
            });
        }
        entries.extend(state.channels.iter().map(|c| LiveEntry {
            key: Key::Channel(c.id),
            pts: c.pts,
            deadline,
            possible_gap: None,
        }));
        entries.sort_by_key(|entry| entry.key);

        getting_diff_for.extend(entries.iter().map(|entry| entry.key));

        Self {
            entries,
            date: state.date,
            seq: state.seq,
            getting_diff_for,
            possible_gaps,
            next_deadline: deadline,
        }
    }

    fn entry(&self, key: Key) -> Option<&LiveEntry> {
        self.entries
            .binary_search_by_key(&key, |entry| entry.key)
            .map(|i| &self.entries[i])
            .ok()
    }

    fn update_entry(&mut self, key: Key, updater: impl FnOnce(&mut LiveEntry)) -> bool {
        match self.entries.binary_search_by_key(&key, |entry| entry.key) {
            Ok(i) => {
                updater(&mut self.entries[i]);
                true
            }
            Err(_) => false,
        }
    }

    fn set_entry(&mut self, entry: LiveEntry) {
        match self
            .entries
            .binary_search_by_key(&entry.key, |entry| entry.key)
        {
            Ok(i) => {
                self.possible_gaps.retain(|&k| k != entry.key);
                self.entries[i] = entry;
            }
            Err(i) => self.entries.insert(i, entry),
        }
    }

    fn set_pts(&mut self, key: Key, pts: i32) {
        if !self.update_entry(key, |entry| entry.pts = pts) {
            self.set_entry(LiveEntry {
                key: key,
                pts: pts,
                deadline: next_updates_deadline(),
                possible_gap: None,
            });
        }
    }

    fn pop_entry(&mut self, key: Key) -> Option<LiveEntry> {
        match self.entries.binary_search_by_key(&key, |entry| entry.key) {
            Ok(i) => Some(self.entries.remove(i)),
            Err(_) => None,
        }
    }

    fn push_gap(&mut self, key: Key, gap: Option<tl::enums::Update>) -> bool {
        let deadline = Instant::now() + POSSIBLE_GAP_TIMEOUT;
        let has_gap = gap.is_some();
        let exists = self.update_entry(key, |entry| {
            let possible_gap = entry.possible_gap.take();

            entry.possible_gap = gap.map(|update| match possible_gap {
                Some(mut possible) => {
                    possible.updates.push(update);
                    possible
                }
                None => PossibleGap {
                    deadline,
                    updates: vec![update],
                },
            });
        });
        if exists {
            if has_gap {
                if !self.possible_gaps.contains(&key) {
                    self.possible_gaps.push(key);
                    self.next_deadline = self.next_deadline.min(deadline);
                }
            } else {
                if let Some(i) = self.possible_gaps.iter().position(|&k| k == key) {
                    self.possible_gaps.remove(i);
                }
            }
        }
        exists
    }

    /// Return the current state in a format that sessions understand.
    ///
    /// This should be used for persisting the state.
    pub fn session_state(&self) -> UpdatesState {
        UpdatesState {
            pts: self.entry(Key::Common).map(|s| s.pts).unwrap_or(NO_PTS),
            qts: self.entry(Key::Secondary).map(|s| s.pts).unwrap_or(NO_PTS),
            date: self.date,
            seq: self.seq,
            channels: self
                .entries
                .iter()
                .filter_map(|entry| match entry.key {
                    Key::Channel(id) => Some(ChannelState { id, pts: entry.pts }.into()),
                    _ => None,
                })
                .collect(),
        }
    }

    /// Return true if the message box is empty and has no state yet.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return the next deadline when receiving updates should timeout.
    ///
    /// If a deadline expired, the corresponding entries will be marked as needing to get its difference.
    /// While there are entries pending of getting their difference, this method returns the current instant.
    pub fn check_deadlines(&mut self) -> Instant {
        let now = Instant::now();

        if !self.getting_diff_for.is_empty() {
            return self.next_deadline;
        }

        if now >= self.next_deadline {
            self.getting_diff_for
                .extend(self.entries.iter().filter_map(|entry| {
                    if now >= entry.effective_deadline() {
                        debug!("deadline for forcibly fetching updates met for {:?}", entry);
                        Some(entry.key)
                    } else {
                        None
                    }
                }));

            // When extending `getting_diff_for`, it's important to have the moral equivalent of
            // `begin_get_diff` (that is, clear possible gaps if we're now getting difference).
            for i in 0..self.getting_diff_for.len() {
                self.push_gap(self.getting_diff_for[i], None);
            }

            // To avoid hogging up all CPU by sleeping for no duration,
            // if the deadline was met but there's no state to fetch,
            // advance the deadline.
            if self.getting_diff_for.is_empty() {
                self.next_deadline = next_updates_deadline();
            }
        }

        self.next_deadline
    }

    /// Reset the deadline of an existing entry.
    /// Does nothing if the entry doesn't exist.
    ///
    /// If the updated deadline was the currently-next-deadline,
    /// the new closest deadline is calculated and stored as the next.
    fn reset_deadline(&mut self, key: Key, deadline: Instant) {
        let mut old_deadline = self.next_deadline;
        self.update_entry(key, |entry| {
            old_deadline = entry.deadline;
            entry.deadline = deadline;
        });
        if self.next_deadline == old_deadline {
            self.next_deadline = self
                .entries
                .iter()
                .fold(deadline, |d, entry| d.min(entry.effective_deadline()));
        }
    }

    fn reset_timeout(&mut self, key: Key, timeout: Option<i32>) {
        self.reset_deadline(
            key,
            timeout
                .map(|t| Instant::now() + Duration::from_secs(t as _))
                .unwrap_or_else(next_updates_deadline),
        );
    }

    /// Sets the update state.
    ///
    /// Should be called right after login if [`MessageBoxes::new`] was used, otherwise undesirable
    /// updates will be fetched.
    ///
    /// Should only be called while [`MessageBoxes::is_empty`].
    pub fn set_state(&mut self, state: tl::enums::updates::State) {
        trace!("setting state {:?}", state);
        debug_assert!(self.is_empty());
        let deadline = next_updates_deadline();
        let state: tl::types::updates::State = state.into();
        self.set_entry(LiveEntry {
            key: Key::Common,
            pts: state.pts,
            deadline,
            possible_gap: None,
        });
        self.set_entry(LiveEntry {
            key: Key::Secondary,
            pts: state.qts,
            deadline,
            possible_gap: None,
        });
        self.date = state.date;
        self.seq = state.seq;
        self.next_deadline = deadline;
    }

    /// Like [`MessageBoxes::set_state`], but for channels. Useful when getting dialogs.
    ///
    /// The update state will only be updated if no entry was known previously.
    pub fn try_set_channel_state(&mut self, id: i64, pts: i32) {
        trace!("trying to set channel state for {}: {}", id, pts);
        if self.entry(Key::Channel(id)).is_none() {
            self.set_entry(LiveEntry {
                key: Key::Channel(id),
                pts: pts,
                deadline: next_updates_deadline(),
                possible_gap: None,
            });
        }
    }

    /// Try to begin getting difference for the given entry.
    /// Fails if the entry does not have a previously-known state that can be used to get its difference.
    ///
    /// Clears any previous gaps.
    fn try_begin_get_diff(&mut self, key: Key) {
        if self.push_gap(key, None) {
            self.getting_diff_for.push(key);
        }
    }

    /// Finish getting difference for the given entry, and resets its deadline.
    /// Does nothing if no difference was being fetched for the given key.
    fn try_end_get_diff(&mut self, key: Key) {
        let i = match self.getting_diff_for.iter().position(|&k| k == key) {
            Some(i) => i,
            None => return,
        };
        self.getting_diff_for.remove(i);
        self.reset_deadline(key, next_updates_deadline());

        debug_assert!(
            self.entry(key)
                .is_none_or(|entry| entry.possible_gap.is_none()),
            "gaps shouldn't be created while getting difference"
        );
    }
}

// "Normal" updates flow (processing and detection of gaps).
impl MessageBoxes {
    /// Process an update and return what should be done with it.
    ///
    /// Updates corresponding to entries for which their difference is currently being fetched
    /// will be ignored. While according to the [updates' documentation]:
    ///
    /// > Implementations \[have\] to postpone updates received via the socket while
    /// > filling gaps in the event and `Update` sequences, as well as avoid filling
    /// > gaps in the same sequence.
    ///
    /// In practice, these updates should have also been retrieved through getting difference.
    ///
    /// [updates' documentation]: https://core.telegram.org/api/updates
    pub fn process_updates(&mut self, updates: UpdatesLike) -> Result<defs::UpdateAndPeers, Gap> {
        trace!("processing updates: {:?}", updates);
        let deadline = next_updates_deadline();

        // Top level, when handling received `updates` and `updatesCombined`.
        // `updatesCombined` groups all the fields we care about, which is why we use it.
        //
        // This assumes all access hashes are already known to the client (socket updates are
        // expected to use `ensure_known_peer_hashes`, and the result from getting difference
        // has to deal with the peers in a different way).
        let tl::types::UpdatesCombined {
            date,
            seq_start,
            seq,
            mut updates,
            users,
            chats,
        } = match adaptor::adapt(updates) {
            Ok(combined) => combined,
            Err(Gap) => {
                self.try_begin_get_diff(Key::Common);
                return Err(Gap);
            }
        };

        let new_date = if date == NO_DATE { self.date } else { date };
        let new_seq = if seq == NO_SEQ { self.seq } else { seq };
        let mk_state = |message_box| State {
            date: new_date,
            seq: new_seq,
            message_box,
        };

        // > For all the other [not `updates` or `updatesCombined`] `Updates` type constructors
        // > there is no need to check `seq` or change a local state.
        if seq_start != NO_SEQ {
            match (self.seq + 1).cmp(&seq_start) {
                // Apply
                Ordering::Equal => {}
                // Ignore
                Ordering::Greater => {
                    debug!(
                        "skipping updates that were already handled at seq = {}",
                        self.seq
                    );
                    return Ok((Vec::new(), users, chats));
                }
                Ordering::Less => {
                    debug!(
                        "gap detected (local seq {}, remote seq {})",
                        self.seq, seq_start
                    );
                    self.try_begin_get_diff(Key::Common);
                    return Err(Gap);
                }
            }
        }

        fn update_sort_key(update: &tl::enums::Update) -> i32 {
            match PtsInfo::from_update(update) {
                Some(info) => info.pts - info.count,
                None => NO_PTS,
            }
        }

        // Telegram can send updates out of order (e.g. `ReadChannelInbox` first
        // and then `NewChannelMessage`, both with the same `pts`, but the `count`
        // is `0` and `1` respectively), so we sort them first.
        updates.sort_by_key(update_sort_key);

        let mut result = Vec::with_capacity(updates.len() + self.possible_gaps.len());
        let had_gaps = !self.possible_gaps.is_empty();

        // This loop does a lot at once to reduce the amount of times we need to iterate over
        // the updates as an optimization.
        //
        // It mutates the local pts state, remembers possible gaps, builds a set of entries for
        // which the deadlines should be reset, and determines whether any local pts was changed
        // so that the seq can be updated too (which could otherwise have been done earlier).
        for update in updates {
            let (key, update) = self.apply_pts_info(update);
            if let Some(key) = key {
                // As soon as we receive an update of any form related to messages (has `PtsInfo`),
                // the "no updates" period for that entry is reset. All the deadlines are reset at
                // once via the temporary entries buffer as an optimization.
                self.reset_deadline(key, deadline);
            }
            if let Some((update, message_box)) = update {
                result.push((update, mk_state(message_box)));
            }
        }

        if had_gaps {
            // Newly-added gaps won't be resolved immediately, so we check if we had gaps before.
            // For each update in possible gaps, see if the gap has been resolved already.
            for i in (0..self.possible_gaps.len()).rev() {
                let key = self.possible_gaps[i];
                let mut gap = None;
                self.update_entry(key, |entry| {
                    gap = entry.possible_gap.take();
                });
                let mut gap = gap.unwrap();
                gap.updates.sort_by_key(update_sort_key);

                // If this fails to apply, it will get re-inserted at the end.
                // All should fail, so the order will be preserved (it would've cycled once).
                for update in gap.updates {
                    if let (_, Some((update, message_box))) = self.apply_pts_info(update) {
                        result.push((update, mk_state(message_box)));
                    }
                }

                // Gap was taken earlier. If it's still empty, the key can be removed from possible gaps.
                if self
                    .entry(key)
                    .is_some_and(|entry| entry.possible_gap.is_none())
                {
                    self.possible_gaps.swap_remove(i);
                    debug!("successfully resolved gap by waiting");
                }
            }
        }

        if !result.is_empty() && self.possible_gaps.is_empty() {
            // > If the updates were applied, local *Updates* state must be updated
            // > with `seq` (unless it's 0) and `date` from the constructor.
            self.date = new_date;
            self.seq = new_seq;
        }

        Ok((result, users, chats))
    }

    /// Tries to apply the input update if its `PtsInfo` follows the correct order.
    ///
    /// If the update can be applied, it is returned; otherwise, the update is stored in a
    /// possible gap (unless it was already handled or would be handled through getting
    /// difference) and `None` is returned.
    fn apply_pts_info(
        &mut self,
        update: tl::enums::Update,
    ) -> (Option<Key>, Option<(tl::enums::Update, Option<MessageBox>)>) {
        if let tl::enums::Update::ChannelTooLong(u) = update {
            self.try_begin_get_diff(Key::Channel(u.channel_id));
            return (None, None);
        }

        let info = match PtsInfo::from_update(&update) {
            Some(info) => info,
            // No pts means that the update can be applied in any order.
            None => return (None, Some((update, None))),
        };

        if self.getting_diff_for.contains(&info.key) {
            debug!(
                "skipping update for {:?} (getting difference, count {:?}, remote {:?})",
                info.key, info.count, info.pts
            );
            // Note: early returning here also prevents gap from being inserted (which they should
            // not be while getting difference).
            return (Some(info.key), Some((update, Some(info.into()))));
        }

        if let Some(local_pts) = self.entry(info.key).map(|entry| entry.pts) {
            match (local_pts + info.count).cmp(&info.pts) {
                // Apply
                Ordering::Equal => {}
                // Ignore
                Ordering::Greater => {
                    debug!(
                        "skipping update for {:?} (local {:?}, count {:?}, remote {:?})",
                        info.key, local_pts, info.count, info.pts
                    );
                    return (Some(info.key), None);
                }
                Ordering::Less => {
                    info!(
                        "gap on update for {:?} (local {:?}, count {:?}, remote {:?})",
                        info.key, local_pts, info.count, info.pts
                    );
                    // TODO store chats too?
                    self.push_gap(info.key, Some(update));

                    return (Some(info.key), None);
                }
            }
        }
        // else, there is no previous `pts` known, and because this update has to be "right"
        // (it's the first one) our `local_pts` must be `pts - pts_count`.

        self.set_pts(info.key, info.pts);
        (Some(info.key), Some((update, Some(info.into()))))
    }
}

/// Getting and applying account difference.
impl MessageBoxes {
    /// Return the request that needs to be made to get the difference, if any.
    pub fn get_difference(&self) -> Option<tl::functions::updates::GetDifference> {
        for entry in [Key::Common, Key::Secondary] {
            if self.getting_diff_for.contains(&entry) {
                let pts = self
                    .entry(Key::Common)
                    .map(|entry| entry.pts)
                    .expect("common entry to exist when getting difference for it");

                let gd = tl::functions::updates::GetDifference {
                    pts,
                    pts_limit: None,
                    pts_total_limit: None,
                    date: self.date.max(1), // non-zero or the request will fail
                    qts: self
                        .entry(Key::Secondary)
                        .map(|entry| entry.pts)
                        .unwrap_or(NO_PTS),
                    qts_limit: None,
                };
                trace!("requesting {:?}", gd);
                return Some(gd);
            }
        }
        None
    }

    /// Similar to [`MessageBoxes::process_updates`], but using the result from getting difference.
    pub fn apply_difference(
        &mut self,
        difference: tl::enums::updates::Difference,
    ) -> defs::UpdateAndPeers {
        trace!("applying account difference: {:?}", difference);
        debug_assert!(
            self.getting_diff_for.contains(&Key::Common)
                || self.getting_diff_for.contains(&Key::Secondary)
        );
        let finish: bool;
        let result = match difference {
            tl::enums::updates::Difference::Empty(diff) => {
                debug!(
                    "handling empty difference (date = {}, seq = {}); no longer getting diff",
                    diff.date, diff.seq
                );
                finish = true;
                self.date = diff.date;
                self.seq = diff.seq;
                (Vec::new(), Vec::new(), Vec::new())
            }
            tl::enums::updates::Difference::Difference(diff) => {
                debug!(
                    "handling full difference {:?}; no longer getting diff",
                    diff.state
                );
                finish = true;
                self.apply_difference_type(diff)
            }
            tl::enums::updates::Difference::Slice(tl::types::updates::DifferenceSlice {
                new_messages,
                new_encrypted_messages,
                other_updates,
                chats,
                users,
                intermediate_state: state,
            }) => {
                debug!("handling partial difference {:?}", state);
                finish = false;
                self.apply_difference_type(tl::types::updates::Difference {
                    new_messages,
                    new_encrypted_messages,
                    other_updates,
                    chats,
                    users,
                    state,
                })
            }
            tl::enums::updates::Difference::TooLong(diff) => {
                debug!(
                    "handling too-long difference (pts = {}); no longer getting diff",
                    diff.pts
                );
                finish = true;
                // the deadline will be reset once the diff ends
                self.set_pts(Key::Common, diff.pts);
                (Vec::new(), Vec::new(), Vec::new())
            }
        };

        if finish {
            self.try_end_get_diff(Key::Common);
            self.try_end_get_diff(Key::Secondary);
        }

        result
    }

    fn apply_difference_type(
        &mut self,
        tl::types::updates::Difference {
            new_messages,
            new_encrypted_messages,
            other_updates: updates,
            chats,
            users,
            state: tl::enums::updates::State::State(state),
        }: tl::types::updates::Difference,
    ) -> defs::UpdateAndPeers {
        self.date = state.date;
        self.seq = state.seq;
        self.set_pts(Key::Common, state.pts);
        self.set_pts(Key::Secondary, state.qts);
        let mk_state = |message_box| State {
            date: state.date,
            seq: state.seq,
            message_box: Some(message_box),
        };

        // other_updates can contain things like UpdateChannelTooLong and UpdateNewChannelMessage.
        // We need to process those as if they were socket updates to discard any we have already handled.
        let us = UpdatesLike::Updates(tl::enums::Updates::Updates(tl::types::Updates {
            updates,
            users,
            chats,
            date: NO_DATE,
            seq: NO_SEQ,
        }));

        // It is possible that the result from `GetDifference` includes users with `min = true`.
        // TODO in that case, we will have to resort to getUsers.
        let (mut result_updates, users, chats) = self
            .process_updates(us)
            .expect("gap is detected while applying difference");

        result_updates.extend(
            new_messages
                .into_iter()
                .map(|message| {
                    (
                        tl::types::UpdateNewMessage {
                            message,
                            pts: NO_PTS,
                            pts_count: 0,
                        }
                        .into(),
                        mk_state(MessageBox::Common { pts: state.pts }),
                    )
                })
                .chain(new_encrypted_messages.into_iter().map(|message| {
                    (
                        tl::types::UpdateNewEncryptedMessage {
                            message,
                            qts: NO_PTS,
                        }
                        .into(),
                        mk_state(MessageBox::Secondary { qts: state.qts }),
                    )
                })),
        );

        (result_updates, users, chats)
    }
}

/// Getting and applying channel difference.
impl MessageBoxes {
    /// Return the request that needs to be made to get a channel's difference, if any.
    ///
    /// Note that this only returns the skeleton of a request.
    /// Both the channel hash and limit will be their default value.
    pub fn get_channel_difference(&self) -> Option<tl::functions::updates::GetChannelDifference> {
        let (key, channel_id) = self.getting_diff_for.iter().find_map(|&key| match key {
            Key::Channel(id) => Some((key, id)),
            _ => None,
        })?;

        if let Some(pts) = self.entry(key).map(|entry| entry.pts) {
            let gd = tl::functions::updates::GetChannelDifference {
                force: false,
                channel: tl::types::InputChannel {
                    channel_id,
                    access_hash: 0,
                }
                .into(),
                filter: tl::enums::ChannelMessagesFilter::Empty,
                pts,
                limit: 0,
            };
            trace!("requesting {:?}", gd);
            Some(gd)
        } else {
            panic!("Should not try to get difference for an entry {key:?} without known state");
        }
    }

    /// Similar to [`MessageBoxes::process_updates`], but using the result from getting difference.
    pub fn apply_channel_difference(
        &mut self,
        difference: tl::enums::updates::ChannelDifference,
    ) -> defs::UpdateAndPeers {
        let (key, channel_id) = self
            .getting_diff_for
            .iter()
            .find_map(|&key| match key {
                Key::Channel(id) => Some((key, id)),
                _ => None,
            })
            .expect("applying channel difference to have a channel in getting_diff_for");

        trace!(
            "applying channel difference for {}: {:?}",
            channel_id, difference
        );

        self.push_gap(key, None);

        let tl::types::updates::ChannelDifference {
            r#final,
            pts,
            timeout,
            new_messages,
            other_updates: updates,
            chats,
            users,
        } = adaptor::adapt_channel_difference(difference);

        if r#final {
            debug!(
                "handling channel {} difference; no longer getting diff",
                channel_id
            );
            self.try_end_get_diff(key);
        } else {
            debug!("handling channel {} difference", channel_id);
        }

        self.set_pts(key, pts);
        let us = UpdatesLike::Updates(tl::enums::Updates::Updates(tl::types::Updates {
            updates,
            users,
            chats,
            date: NO_DATE,
            seq: NO_SEQ,
        }));
        let (mut result_updates, users, chats) = self
            .process_updates(us)
            .expect("gap is detected while applying channel difference");

        result_updates.extend(new_messages.into_iter().map(|message| {
            (
                tl::types::UpdateNewChannelMessage {
                    message,
                    pts: NO_PTS,
                    pts_count: 0,
                }
                .into(),
                State {
                    date: self.date,
                    seq: self.seq,
                    message_box: Some(MessageBox::Channel { channel_id, pts }),
                },
            )
        }));

        self.reset_timeout(key, timeout);

        (result_updates, users, chats)
    }

    pub fn end_channel_difference(&mut self, reason: PrematureEndReason) {
        let (key, channel_id) = self
            .getting_diff_for
            .iter()
            .find_map(|&key| match key {
                Key::Channel(id) => Some((key, id)),
                _ => None,
            })
            .expect("ending channel difference to have a channel in getting_diff_for");

        trace!(
            "ending channel difference for {} because {:?}",
            channel_id, reason
        );
        match reason {
            PrematureEndReason::TemporaryServerIssues => {
                self.push_gap(key, None);
                self.try_end_get_diff(key);
            }
            PrematureEndReason::Banned => {
                self.push_gap(key, None);
                self.try_end_get_diff(key);
                self.pop_entry(key);
            }
        }
    }
}

/// Reason for calling [`MessageBoxes::end_channel_difference`].
#[derive(Debug)]
pub enum PrematureEndReason {
    /// The channel difference failed to be fetched for a temporary reasons.
    ///
    /// The channel state must be kept,
    /// and getting its difference retried in the future.
    TemporaryServerIssues,
    /// The logged-in account has been banned from the channel and won't
    /// be able to fetch the difference in the future.
    ///
    /// The channel state will be removed,
    /// and getting its difference should not be retried in the future.
    Banned,
}
