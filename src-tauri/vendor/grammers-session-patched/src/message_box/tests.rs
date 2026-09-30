// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use super::defs::{
    Gap, MessageBox, MessageBoxes, NO_DATE, NO_PTS, NO_SEQ, NO_UPDATES_TIMEOUT, State,
    UpdateAndPeers,
};
use super::{PrematureEndReason, next_updates_deadline};
use crate::defs::{ChannelState, UpdatesState};
use crate::message_box::POSSIBLE_GAP_TIMEOUT;
use crate::message_box::defs::UpdatesLike;
use grammers_tl_types as tl;
use std::cell::RefCell;
use std::ops::Add;
use std::time::Duration;

thread_local! {
    static NOW: RefCell<Instant> = RefCell::new(Instant(Duration::ZERO));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant(Duration);

impl Instant {
    pub fn now() -> Self {
        NOW.with_borrow(|now| *now)
    }
}

impl Add<Duration> for Instant {
    type Output = Self;

    fn add(self, rhs: Duration) -> Self::Output {
        Self(self.0 + rhs)
    }
}

fn reset_time() {
    NOW.with_borrow_mut(|now| now.0 = Duration::ZERO);
}

fn advance_time_by(duration: Duration) {
    NOW.with_borrow_mut(|now| now.0 += duration);
}

fn state(date: i32, seq: i32, pts: i32, qts: i32) -> tl::enums::updates::State {
    tl::enums::updates::State::State(tl::types::updates::State {
        pts,
        qts,
        date,
        seq,
        unread_count: 0,
    })
}

fn update(pts: i32) -> tl::enums::Update {
    tl::enums::Update::DeleteMessages(tl::types::UpdateDeleteMessages {
        messages: Vec::new(),
        pts,
        pts_count: 1,
    })
}

fn updates(date: i32, seq: i32, pts: i32) -> UpdatesLike {
    UpdatesLike::Updates(tl::enums::Updates::Updates(tl::types::Updates {
        updates: vec![update(pts)],
        users: Vec::new(),
        chats: Vec::new(),
        date,
        seq,
    }))
}

fn updates_ok(date: i32, seq: i32, pts: i32) -> Result<UpdateAndPeers, Gap> {
    Ok((
        vec![(
            update(pts),
            State {
                date,
                seq,
                message_box: Some(MessageBox::Common { pts }),
            },
        )],
        Vec::new(),
        Vec::new(),
    ))
}

fn merge_updates(updates: Vec<Result<UpdateAndPeers, Gap>>) -> Result<UpdateAndPeers, Gap> {
    updates
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map(|updates| {
            updates
                .into_iter()
                .fold((Vec::new(), Vec::new(), Vec::new()), |mut result, tuple| {
                    result.0.extend(tuple.0);
                    result.1.extend(tuple.1);
                    result.2.extend(tuple.2);
                    result
                })
        })
}

fn get_difference(date: i32, pts: i32, qts: i32) -> tl::functions::updates::GetDifference {
    tl::functions::updates::GetDifference {
        pts,
        pts_limit: None,
        pts_total_limit: None,
        date,
        qts,
        qts_limit: None,
    }
}

fn get_channel_difference(id: i64, pts: i32) -> tl::functions::updates::GetChannelDifference {
    tl::functions::updates::GetChannelDifference {
        force: false,
        channel: tl::enums::InputChannel::Channel(tl::types::InputChannel {
            channel_id: id,
            access_hash: 0,
        }),
        filter: tl::enums::ChannelMessagesFilter::Empty,
        pts,
        limit: 0,
    }
}

#[test]
fn test_connect_flow_empty() {
    reset_time();
    let state = UpdatesState {
        pts: NO_PTS,
        qts: NO_PTS,
        date: NO_DATE,
        seq: NO_SEQ,
        channels: Vec::new(),
    };
    let message_boxes = MessageBoxes::load(state.clone());

    assert!(message_boxes.is_empty());
    assert_eq!(message_boxes.get_difference(), None);
    assert_eq!(message_boxes.get_channel_difference(), None);
    assert_eq!(message_boxes.session_state(), state)
}

#[test]
fn test_connect_flow_with_data() {
    reset_time();
    let state = UpdatesState {
        pts: 12,
        qts: 34,
        date: 56,
        seq: 78,
        channels: vec![ChannelState { id: 43, pts: 21 }],
    };
    let message_boxes = MessageBoxes::load(state.clone());

    assert!(!message_boxes.is_empty());
    assert_eq!(
        message_boxes.get_difference(),
        Some(get_difference(56, 12, 34))
    );
    assert_eq!(
        message_boxes.get_channel_difference(),
        Some(get_channel_difference(43, 21))
    );
    assert_eq!(message_boxes.session_state(), state)
}

#[test]
fn test_complete_login_flow() {
    reset_time();
    let mut message_boxes = MessageBoxes::new();
    assert!(message_boxes.is_empty());

    message_boxes.set_state(state(56, 78, 12, 34));
    assert!(!message_boxes.is_empty());
    assert_eq!(message_boxes.get_difference(), None);
    assert_eq!(message_boxes.get_channel_difference(), None);
    assert_eq!(
        message_boxes.session_state(),
        UpdatesState {
            pts: 12,
            qts: 34,
            date: 56,
            seq: 78,
            channels: Vec::new()
        }
    )
}

#[test]
fn test_iter_dialogs_flow() {
    reset_time();
    let mut message_boxes = MessageBoxes::new();

    message_boxes.try_set_channel_state(98, 76);
    message_boxes.try_set_channel_state(54, 32);
    message_boxes.try_set_channel_state(98, 10);

    assert_eq!(
        message_boxes.session_state(),
        UpdatesState {
            channels: vec![
                // Notably: sorted, and only the first set is kept.
                ChannelState { id: 54, pts: 32 }.into(),
                ChannelState { id: 98, pts: 76 }.into(),
            ],
            ..message_boxes.session_state()
        }
    )
}

#[test]
fn test_next_raw_update_flow_empty() {
    reset_time();
    let mut message_boxes = MessageBoxes::new();

    let deadline = next_updates_deadline();
    assert_eq!(message_boxes.check_deadlines(), deadline);
    assert_eq!(message_boxes.get_difference(), None);
    assert_eq!(message_boxes.get_channel_difference(), None);

    advance_time_by(NO_UPDATES_TIMEOUT / 2); // assert unchanged since previous deadline was not met yet
    assert_eq!(message_boxes.check_deadlines(), deadline);
    assert_eq!(message_boxes.get_difference(), None);
    assert_eq!(message_boxes.get_channel_difference(), None);

    advance_time_by(NO_UPDATES_TIMEOUT); // assert changed to avoid sleeping with no delay
    assert_eq!(message_boxes.check_deadlines(), next_updates_deadline());
    assert_eq!(message_boxes.get_difference(), None);
    assert_eq!(message_boxes.get_channel_difference(), None);

    advance_time_by(NO_UPDATES_TIMEOUT + Duration::from_secs(1)); // assert change based on now, not last deadline
    assert_eq!(message_boxes.check_deadlines(), next_updates_deadline());
    assert_eq!(message_boxes.get_difference(), None);
    assert_eq!(message_boxes.get_channel_difference(), None);
}

#[test]
fn test_next_raw_update_flow_common_timeout() {
    reset_time();
    let mut message_boxes = MessageBoxes::new();
    message_boxes.set_state(state(56, 78, 12, 34));

    advance_time_by(NO_UPDATES_TIMEOUT);

    assert_eq!(message_boxes.get_difference(), None);
    assert_eq!(message_boxes.check_deadlines(), Instant::now());
    assert_eq!(
        message_boxes.get_difference(),
        Some(get_difference(56, 12, 34))
    );

    message_boxes
        .apply_difference(tl::types::updates::DifferenceEmpty { date: 90, seq: 91 }.into());

    assert_eq!(message_boxes.get_difference(), None);
    assert_eq!(message_boxes.check_deadlines(), next_updates_deadline());
    assert_eq!(
        message_boxes.session_state(),
        UpdatesState {
            pts: 12,
            qts: 34,
            date: 90,
            seq: 91,
            channels: Vec::new()
        }
    );
}

#[test]
fn test_next_raw_update_flow_channel_timeout() {
    reset_time();
    let mut message_boxes = MessageBoxes::new();
    message_boxes.try_set_channel_state(12, 34);

    advance_time_by(NO_UPDATES_TIMEOUT);

    assert_eq!(message_boxes.get_channel_difference(), None);
    assert_eq!(message_boxes.check_deadlines(), Instant::now());
    assert_eq!(
        message_boxes.get_channel_difference(),
        Some(get_channel_difference(12, 34))
    );

    message_boxes.apply_channel_difference(
        tl::types::updates::ChannelDifferenceEmpty {
            r#final: true,
            pts: 56,
            timeout: None,
        }
        .into(),
    );

    assert_eq!(message_boxes.get_difference(), None);
    assert_eq!(message_boxes.check_deadlines(), next_updates_deadline());
    assert_eq!(
        message_boxes.session_state(),
        UpdatesState {
            channels: vec![ChannelState { id: 12, pts: 56 }.into()],
            ..message_boxes.session_state()
        }
    );
}

#[test]
fn test_next_raw_update_flow_channel_issues() {
    reset_time();
    let mut message_boxes = MessageBoxes::new();
    message_boxes.try_set_channel_state(12, 34);

    advance_time_by(NO_UPDATES_TIMEOUT);
    assert_eq!(message_boxes.check_deadlines(), Instant::now());
    assert!(message_boxes.get_channel_difference().is_some());

    message_boxes.end_channel_difference(PrematureEndReason::TemporaryServerIssues);
    assert!(message_boxes.get_channel_difference().is_none());
    assert_eq!(
        message_boxes.session_state(),
        UpdatesState {
            channels: vec![ChannelState { id: 12, pts: 34 }.into()],
            ..message_boxes.session_state()
        }
    );
}

#[test]
fn test_next_raw_update_flow_channel_ban() {
    reset_time();
    let mut message_boxes = MessageBoxes::new();
    message_boxes.try_set_channel_state(12, 34);

    advance_time_by(NO_UPDATES_TIMEOUT);
    assert_eq!(message_boxes.check_deadlines(), Instant::now());
    assert!(message_boxes.get_channel_difference().is_some());

    message_boxes.end_channel_difference(PrematureEndReason::Banned);
    assert!(message_boxes.get_channel_difference().is_none());
    assert_eq!(
        message_boxes.session_state(),
        UpdatesState {
            channels: vec![],
            ..message_boxes.session_state()
        }
    );
}

#[test]
fn test_next_raw_update_flow_no_new_get_diff_if_already_fetching() {
    reset_time();
    let mut message_boxes = MessageBoxes::new();

    // t(0) = first channel registers timeout at t(3)
    message_boxes.try_set_channel_state(11, 12);
    let expected_first_deadline = next_updates_deadline();

    // t(2) = second and third channel register timeout at t(5)
    advance_time_by(2 * (NO_UPDATES_TIMEOUT / 3));
    message_boxes.try_set_channel_state(21, 22);
    message_boxes.try_set_channel_state(31, 32);
    let expected_second_deadline = next_updates_deadline();

    // t(4) = first channel times out
    advance_time_by(2 * (NO_UPDATES_TIMEOUT / 3));
    assert_eq!(message_boxes.check_deadlines(), expected_first_deadline);
    assert_eq!(
        message_boxes.get_channel_difference(),
        Some(get_channel_difference(11, 12))
    );

    // t(6) = second and third channel should have timed out by now, but didn't yet
    advance_time_by(2 * (NO_UPDATES_TIMEOUT / 3));
    message_boxes.end_channel_difference(PrematureEndReason::TemporaryServerIssues);
    assert_eq!(message_boxes.get_channel_difference(), None);

    // t(6) = checking deadlines now triggers difference for second and third
    assert_eq!(message_boxes.check_deadlines(), expected_second_deadline);
    assert_eq!(
        message_boxes.get_channel_difference(),
        Some(get_channel_difference(21, 22))
    );
    message_boxes.end_channel_difference(PrematureEndReason::TemporaryServerIssues);
    assert_eq!(
        message_boxes.get_channel_difference(),
        Some(get_channel_difference(31, 32))
    );
    message_boxes.end_channel_difference(PrematureEndReason::TemporaryServerIssues);
    assert_eq!(message_boxes.get_channel_difference(), None);
}

#[test]
fn test_process_socket_updates_flow_already_processed() {
    reset_time();
    let mut message_boxes = MessageBoxes::new();

    message_boxes.set_state(state(12, 34, 56, 78));

    for (seq, pts) in [
        (33, 57),     // seq already applied (<)
        (34, 57),     // seq already applied (=)
        (35, 55),     // seq ok (>), pts already applied (<)
        (35, 56),     // seq ok (>), pts already applied (=)
        (NO_PTS, 55), // seq ok (0), pts already applied (<)
        (NO_PTS, 56), // seq ok (0), pts already applied (=)
    ] {
        assert_eq!(
            message_boxes.process_updates(updates(13, seq, pts).into()), // date doesn't matter
            Ok((Vec::new(), Vec::new(), Vec::new()))
        );
    }
}

#[test]
fn test_process_socket_updates_flow_common_already_applied() {
    reset_time();
    let mut message_boxes = MessageBoxes::new();

    message_boxes.set_state(state(12, 34, 56, 78));

    for (seq, pts) in [
        (33, 57),     // seq already applied (<)
        (34, 57),     // seq already applied (=)
        (35, 55),     // seq ok (>), pts already applied (<)
        (35, 56),     // seq ok (>), pts already applied (=)
        (NO_PTS, 55), // seq ok (0), pts already applied (<)
        (NO_PTS, 56), // seq ok (0), pts already applied (=)
    ] {
        assert_eq!(
            message_boxes.process_updates(updates(13, seq, pts).into()), // date doesn't matter
            Ok((Vec::new(), Vec::new(), Vec::new()))
        );
    }
}

#[test]
fn test_process_socket_updates_flow_common_difference_ok() {
    reset_time();
    let mut message_boxes = MessageBoxes::new();

    message_boxes.set_state(state(12, 34, 56, 78));
    message_boxes.try_begin_get_diff(super::Key::Common);

    assert_eq!(
        message_boxes.get_difference(),
        Some(get_difference(12, 56, 78))
    );
    assert_eq!(
        message_boxes.process_updates(updates(NO_DATE, NO_SEQ, 57)),
        updates_ok(12, 34, 57)
    );
}

#[test]
fn test_process_socket_updates_flow_common_ok() {
    reset_time();
    let mut message_boxes = MessageBoxes::new();

    message_boxes.set_state(state(12, 34, 56, 78));

    assert_eq!(
        message_boxes.process_updates(updates(NO_DATE, NO_SEQ, 57)),
        updates_ok(12, 34, 57)
    );
    assert_eq!(
        message_boxes.process_updates(updates(NO_DATE, 35, 58)),
        updates_ok(12, 35, 58)
    );
    assert_eq!(
        message_boxes.process_updates(updates(13, 36, 59)),
        updates_ok(13, 36, 59)
    );
    assert_eq!(
        message_boxes.process_updates(updates(14, NO_SEQ, 60)),
        updates_ok(14, 36, 60)
    );
}

#[test]
fn test_process_socket_updates_flow_common_seq_gap() {
    reset_time();
    let mut message_boxes = MessageBoxes::new();

    message_boxes.set_state(state(12, 34, 56, 78));

    assert_eq!(message_boxes.process_updates(updates(13, 36, 57)), Err(Gap));
    assert_eq!(
        message_boxes.get_difference(),
        Some(get_difference(12, 56, 78))
    );
}

#[test]
fn test_process_socket_updates_flow_common_pts_possible_gap_resolves() {
    reset_time();
    let mut message_boxes = MessageBoxes::new();

    message_boxes.set_state(state(12, 34, 56, 78));

    // No additional updates before gap resolves.
    assert_eq!(
        message_boxes.process_updates(updates(NO_DATE, NO_SEQ, 58)),
        Ok((Vec::new(), Vec::new(), Vec::new()))
    );
    advance_time_by(POSSIBLE_GAP_TIMEOUT / 2);
    message_boxes.check_deadlines();
    assert_eq!(
        message_boxes.process_updates(updates(NO_DATE, NO_SEQ, 57)),
        merge_updates(vec![updates_ok(12, 34, 57), updates_ok(12, 34, 58)])
    );

    // One additional update before gap resolves.
    assert_eq!(
        message_boxes.process_updates(updates(NO_DATE, NO_SEQ, 61)),
        Ok((Vec::new(), Vec::new(), Vec::new()))
    );
    advance_time_by(POSSIBLE_GAP_TIMEOUT / 4);
    message_boxes.check_deadlines();
    assert_eq!(
        message_boxes.process_updates(updates(NO_DATE, NO_SEQ, 60)),
        Ok((Vec::new(), Vec::new(), Vec::new()))
    );
    advance_time_by(POSSIBLE_GAP_TIMEOUT / 4);
    message_boxes.check_deadlines();
    assert_eq!(
        message_boxes.process_updates(updates(NO_DATE, NO_SEQ, 59)),
        merge_updates(vec![
            updates_ok(12, 34, 59),
            updates_ok(12, 34, 60),
            updates_ok(12, 34, 61),
        ])
    );
}

#[test]
fn test_process_socket_updates_flow_common_pts_gap() {
    reset_time();
    let mut message_boxes = MessageBoxes::new();

    message_boxes.set_state(state(12, 34, 56, 78));

    assert_eq!(
        message_boxes.process_updates(updates(NO_DATE, NO_SEQ, 58)),
        Ok((Vec::new(), Vec::new(), Vec::new()))
    );
    advance_time_by(3 * (POSSIBLE_GAP_TIMEOUT / 2));
    assert_eq!(message_boxes.get_difference(), None);
    message_boxes.check_deadlines();
    assert_eq!(
        message_boxes.get_difference(),
        Some(get_difference(12, 56, 78))
    );
}
