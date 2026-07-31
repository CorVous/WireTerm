//! Session-only Playlist playback state machine.

use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

use crate::playlist::{ItemId, PlaylistItem, PlaylistRevision};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActiveWork {
    Rendering,
    Sending,
}

#[derive(Clone, Debug)]
enum Phase {
    Boundary,
    Active {
        work: ActiveWork,
        turn: Turn,
    },
    Interval(Turn),
    WaitingReconnect {
        item_id: ItemId,
        former_index: usize,
    },
    Cooldown {
        until: Instant,
    },
}

#[derive(Clone, Debug)]
struct Turn {
    item: PlaylistItem,
    former_index: usize,
    started: Instant,
    interval: Duration,
    revision: u64,
}

#[derive(Clone, Debug)]
pub struct TurnRequest {
    pub item: PlaylistItem,
    pub started: Instant,
    pub interval: Duration,
    pub revision: u64,
}

/// Pure state machine. Rendering and the exclusive `HostBridge` remain owned
/// by the application and report completion back through the methods below.
pub struct PlaybackController {
    running: bool,
    step_once: bool,
    phase: Phase,
    current_item: Option<ItemId>,
    current_former_index: usize,
    failures_this_pass: BTreeSet<ItemId>,
}

impl Default for PlaybackController {
    fn default() -> Self {
        Self::new_running()
    }
}

impl PlaybackController {
    #[must_use]
    pub const fn new_running() -> Self {
        Self {
            running: true,
            step_once: false,
            phase: Phase::Boundary,
            current_item: None,
            current_former_index: 0,
            failures_this_pass: BTreeSet::new(),
        }
    }

    #[must_use]
    pub const fn is_running(&self) -> bool {
        self.running
    }

    #[must_use]
    pub const fn current_item(&self) -> Option<ItemId> {
        self.current_item
    }

    #[must_use]
    pub const fn active_work(&self) -> Option<ActiveWork> {
        match self.phase {
            Phase::Active { work, .. } => Some(work),
            _ => None,
        }
    }

    #[must_use]
    pub const fn can_next(&self) -> bool {
        matches!(self.phase, Phase::Interval(_)) || !self.running
    }

    #[must_use]
    pub const fn is_waiting_for_reconnect(&self) -> bool {
        matches!(self.phase, Phase::WaitingReconnect { .. })
    }

    #[must_use]
    pub fn interval_progress(&self, now: Instant) -> Option<f32> {
        let Phase::Interval(turn) = &self.phase else {
            return None;
        };
        let total = turn.interval.as_secs_f32();
        if total <= f32::EPSILON {
            return Some(1.0);
        }
        Some((now.duration_since(turn.started).as_secs_f32() / total).clamp(0.0, 1.0))
    }

    pub const fn pause(&mut self) {
        self.running = false;
        self.step_once = false;
    }

    pub fn resume(&mut self) {
        if matches!(self.phase, Phase::Interval(_)) {
            self.phase = Phase::Boundary;
        }
        self.running = true;
        self.step_once = false;
    }

    pub fn advance_next(&mut self) -> bool {
        if !self.can_next() {
            return false;
        }
        self.phase = Phase::Boundary;
        if !self.running {
            self.step_once = true;
        }
        true
    }

    /// Return the next immutable turn when playback is ready to begin work.
    ///
    /// The caller passes the latest saved Playlist revision. It is consulted
    /// only at a boundary, so applied edits cannot mutate an active turn.
    pub fn poll(
        &mut self,
        playlist: &PlaylistRevision,
        now: Instant,
        connected: bool,
    ) -> Option<TurnRequest> {
        if !(self.running || self.step_once) {
            return None;
        }

        let reconnect_item = match self.phase {
            Phase::WaitingReconnect {
                item_id,
                former_index,
            } if connected => Some((item_id, former_index)),
            _ => None,
        };
        match &self.phase {
            Phase::Active { .. } => return None,
            Phase::Interval(turn) if now < turn.started + turn.interval => return None,
            Phase::Interval(_) => self.phase = Phase::Boundary,
            Phase::Cooldown { until } if now < *until => return None,
            Phase::Cooldown { .. } => {
                self.failures_this_pass.clear();
                self.phase = Phase::Boundary;
            }
            Phase::WaitingReconnect { .. } if !connected => return None,
            Phase::WaitingReconnect {
                item_id,
                former_index,
            } => {
                self.current_item = Some(*item_id);
                self.current_former_index = *former_index;
                self.phase = Phase::Boundary;
            }
            Phase::Boundary => {}
        }

        let candidate = reconnect_item
            .and_then(|(item_id, _)| {
                playlist
                    .items
                    .iter()
                    .enumerate()
                    .find(|(_, item)| item.id == item_id && item.enabled)
            })
            .or_else(|| {
                select_boundary_item(
                    playlist,
                    self.current_item,
                    self.current_former_index,
                    self.current_item.is_none(),
                )
            });
        let Some((index, item)) = candidate else {
            self.current_item = None;
            return None;
        };
        self.current_item = Some(item.id);
        self.current_former_index = index;
        if !connected {
            self.phase = Phase::WaitingReconnect {
                item_id: item.id,
                former_index: index,
            };
            return None;
        }

        let turn = Turn {
            interval: Duration::from_secs(
                u64::from(playlist.effective_interval_minutes(item)) * 60,
            ),
            item: item.clone(),
            former_index: index,
            started: now,
            revision: playlist.revision,
        };
        let request = TurnRequest {
            item: turn.item.clone(),
            started: turn.started,
            interval: turn.interval,
            revision: turn.revision,
        };
        self.phase = Phase::Active {
            work: ActiveWork::Rendering,
            turn,
        };
        Some(request)
    }

    pub fn rendered(&mut self) {
        let Phase::Active { work, turn } = &mut self.phase else {
            return;
        };
        if *work == ActiveWork::Rendering {
            *work = ActiveWork::Sending;
            let _ = turn;
        }
    }

    pub fn send_succeeded(&mut self) {
        let Phase::Active { turn, .. } = &self.phase else {
            return;
        };
        let turn = turn.clone();
        self.failures_this_pass.clear();
        if self.step_once {
            self.step_once = false;
        }
        self.phase = Phase::Interval(turn);
    }

    pub fn failed(&mut self, playlist: &PlaylistRevision, now: Instant, disconnected: bool) {
        let (item_id, former_index) = match &self.phase {
            Phase::Active { turn, .. } => (turn.item.id, turn.former_index),
            Phase::WaitingReconnect {
                item_id,
                former_index,
            } => (*item_id, *former_index),
            _ => return,
        };
        self.current_item = Some(item_id);
        self.current_former_index = former_index;
        if disconnected {
            self.phase = Phase::WaitingReconnect {
                item_id,
                former_index,
            };
            return;
        }

        self.failures_this_pass.insert(item_id);
        if self.step_once {
            self.step_once = false;
            self.phase = Phase::Boundary;
            return;
        }
        let enabled_count = playlist.items.iter().filter(|item| item.enabled).count();
        if enabled_count > 0 && self.failures_this_pass.len() >= enabled_count {
            self.phase = Phase::Cooldown {
                until: now + Duration::from_secs(u64::from(playlist.default_interval_minutes) * 60),
            };
        } else {
            self.phase = Phase::Boundary;
        }
    }
}

fn select_boundary_item(
    playlist: &PlaylistRevision,
    current: Option<ItemId>,
    former_index: usize,
    first_turn: bool,
) -> Option<(usize, &PlaylistItem)> {
    if first_turn {
        return playlist
            .items
            .iter()
            .enumerate()
            .find(|(_, item)| item.enabled);
    }
    if let Some(current) = current
        && let Some(index) = playlist.items.iter().position(|item| item.id == current)
    {
        return playlist
            .items
            .iter()
            .enumerate()
            .skip(index + 1)
            .chain(playlist.items.iter().enumerate())
            .find(|(_, item)| item.enabled);
    }
    playlist
        .items
        .iter()
        .enumerate()
        .skip(former_index.min(playlist.items.len()))
        .chain(playlist.items.iter().enumerate())
        .find(|(_, item)| item.enabled)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::playlist::PlaylistSource;

    use super::*;

    fn playlist(count: usize) -> PlaylistRevision {
        let mut playlist = PlaylistRevision::default();
        for index in 0..count {
            playlist.add_item(
                format!("Item {index}"),
                PlaylistSource::Image {
                    path: PathBuf::from(format!("{index}.png")),
                },
            );
        }
        playlist
    }

    #[test]
    fn interval_is_start_to_start_and_edits_wait_for_boundary() {
        let mut playlist = playlist(2);
        playlist.default_interval_minutes = 1;
        playlist.revision = 1;
        let now = Instant::now();
        let mut playback = PlaybackController::new_running();
        let first = playback.poll(&playlist, now, true).expect("first turn");
        assert_eq!(first.item.title, "Item 0");
        playback.rendered();
        playback.send_succeeded();

        playlist.items.swap(0, 1);
        playlist.revision = 2;
        assert!(
            playback
                .poll(&playlist, now + Duration::from_secs(59), true)
                .is_none()
        );
        let next = playback
            .poll(&playlist, now + Duration::from_mins(1), true)
            .expect("boundary");
        assert_eq!(next.revision, 2);
        assert_eq!(next.item.title, "Item 1");
    }

    #[test]
    fn disconnect_retries_same_identity_as_fresh_turn() {
        let playlist = playlist(2);
        let now = Instant::now();
        let mut playback = PlaybackController::new_running();
        let first = playback.poll(&playlist, now, true).expect("first");
        playback.failed(&playlist, now, true);
        assert!(playback.is_waiting_for_reconnect());
        assert!(
            playback
                .poll(&playlist, now + Duration::from_secs(30), false)
                .is_none()
        );
        let retry = playback
            .poll(&playlist, now + Duration::from_mins(1), true)
            .expect("retry");
        assert_eq!(retry.item.id, first.item.id);
        assert_eq!(retry.started, now + Duration::from_mins(1));
    }

    #[test]
    fn pause_resume_and_next_follow_session_rules() {
        let playlist = playlist(2);
        let now = Instant::now();
        let mut playback = PlaybackController::new_running();
        playback.poll(&playlist, now, true).expect("first");
        playback.rendered();
        playback.send_succeeded();
        playback.pause();
        assert!(playback.can_next());
        assert!(playback.advance_next());
        let stepped = playback
            .poll(&playlist, now + Duration::from_secs(1), true)
            .expect("one paused turn");
        assert_eq!(stepped.item.title, "Item 1");
        playback.rendered();
        playback.send_succeeded();
        assert!(
            playback
                .poll(&playlist, now + Duration::from_secs(2), true)
                .is_none()
        );

        playback.resume();
        let resumed = playback
            .poll(&playlist, now + Duration::from_secs(3), true)
            .expect("resume advances immediately");
        assert_eq!(resumed.item.title, "Item 0");
    }

    #[test]
    fn all_failures_wait_the_default_interval() {
        let mut playlist = playlist(2);
        playlist.default_interval_minutes = 1;
        let now = Instant::now();
        let mut playback = PlaybackController::new_running();
        playback.poll(&playlist, now, true).expect("first");
        playback.failed(&playlist, now, false);
        playback
            .poll(&playlist, now + Duration::from_secs(1), true)
            .expect("second");
        playback.failed(&playlist, now + Duration::from_secs(1), false);
        assert!(
            playback
                .poll(&playlist, now + Duration::from_mins(1), true)
                .is_none()
        );
        assert!(
            playback
                .poll(&playlist, now + Duration::from_secs(61), true)
                .is_some()
        );
    }
}
