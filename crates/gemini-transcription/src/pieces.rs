//! Sample-exact pause partitioning and conservative quota scheduling. VAD scores
//! are supplied by the app's one-thread Silero worker, never the capture thread.
use std::collections::VecDeque;

pub const RATE: usize = 16_000;
pub const FRAME: usize = 512;
/// Only detected pauses are eligible boundaries. A target is a minimum, never
/// a deadline: if speech continues we keep the audio instead of guessing a cut.
pub struct Cutter {
    since: usize,
    quiet: usize,
    target: usize,
}
impl Default for Cutter {
    fn default() -> Self {
        Self::new(60)
    }
}
impl Cutter {
    pub fn new(target_seconds: u32) -> Self {
        Self {
            since: 0,
            quiet: 0,
            target: target_seconds.clamp(30, 90) as usize * RATE,
        }
    }
    /// Return the end of at least 0.4 seconds of detected non-speech, after
    /// the configured target. Every sample, including silence, stays in order.
    /// VAD can be wrong; we never add a second energy-based forced-cut guess.
    pub fn push(&mut self, frame: &[f32], speech: bool) -> Option<usize> {
        self.since += frame.len();
        self.quiet = if speech { 0 } else { self.quiet + frame.len() };
        if self.since >= self.target && self.quiet >= RATE * 4 / 10 {
            let cut = self.since;
            self.since = 0;
            self.quiet = 0;
            return Some(cut);
        }
        None
    }
}
/// A rolling window, with no guessed requests/minute. Audio accumulates while
/// the window is full; callers merge it, rather than enqueue individual pieces.
#[derive(Default)]
pub struct Limiter {
    sent: VecDeque<u64>,
    pub minute: Option<u32>,
    pub daily: Option<u32>,
}
impl Limiter {
    pub fn new(minute: Option<u32>, daily: Option<u32>) -> Self {
        Self {
            minute,
            daily,
            ..Self::default()
        }
    }
    pub fn allow(&mut self, now: u64, day_count: u32) -> bool {
        self.allow_with_headroom(now, day_count, 0)
    }
    /// During capture keep one request available for the final tail or an
    /// explicit retry. Finalization/manual recovery can use that capacity.
    pub fn allow_with_headroom(&mut self, now: u64, day_count: u32, headroom: u32) -> bool {
        while self
            .sent
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= 60_000)
        {
            self.sent.pop_front();
        }
        self.minute.is_some_and(|n| {
            n > 0 && self.sent.len().saturating_add(headroom as usize) < n as usize
        }) && self
            .daily
            .is_some_and(|n| n > day_count.saturating_add(headroom))
    }
    pub fn record(&mut self, now: u64) {
        self.sent.push_back(now);
    }
}
pub fn pacific_day() -> String {
    chrono::Utc::now()
        .with_timezone(&chrono_tz::America::Los_Angeles)
        .format("%Y-%m-%d")
        .to_string()
}
pub fn last_sentence(text: &str) -> String {
    let trimmed = text.trim_end_matches(['.', '!', '?', ' ', '\n']);
    let tail = trimmed
        .rsplit(['.', '!', '?', '\n'])
        .next()
        .unwrap_or(trimmed)
        .trim();
    tail.chars().take(1000).collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pause_cuts_preserve_every_sample_without_overlap() {
        let input: Vec<_> = (0..RATE * 62 + 3).map(|i| i as f32).collect();
        let mut cutter = Cutter::default();
        let mut pending = Vec::new();
        let mut output = Vec::new();
        for frame in input.chunks(FRAME) {
            pending.extend_from_slice(frame);
            if let Some(n) = cutter.push(frame, false) {
                output.extend(pending.drain(..n));
            }
        }
        output.extend(pending);
        assert_eq!(output, input);
    }
    #[test]
    fn targets_wait_for_a_pause_and_never_cut_on_low_energy_speech() {
        for target in [30, 60, 90] {
            let mut c = Cutter::new(target);
            let speech_frames = ((target + 15) as usize * RATE).div_ceil(FRAME);
            for _ in 0..speech_frames {
                assert_eq!(c.push(&[0.001; FRAME], true), None);
            }
            for _ in 0..12 {
                // 0.384 seconds is too short.
                assert_eq!(c.push(&[0.0; FRAME], false), None);
            }
            assert_eq!(
                c.push(&[0.0; FRAME], false),
                Some((speech_frames + 13) * FRAME)
            );
        }
    }
    #[test]
    fn brief_pauses_before_target_do_not_send_and_partition_preserves_repetitions() {
        for target in [30, 60, 90] {
            let mut c = Cutter::new(target);
            let mut pending = Vec::new();
            let mut output = Vec::new();
            let mut original = Vec::new();
            let frames = ((target as usize + 4) * RATE).div_ceil(FRAME);
            for i in 0..frames {
                // Identical audio motifs represent intentional repetition.
                let frame = vec![if i % 20 < 13 { 0.0 } else { 0.5 }; FRAME];
                original.extend_from_slice(&frame);
                pending.extend_from_slice(&frame);
                if let Some(n) = c.push(&frame, i % 20 >= 13) {
                    assert!(n >= target as usize * RATE);
                    output.extend(pending.drain(..n));
                }
            }
            output.extend(pending);
            assert_eq!(output, original);
        }
    }
    #[test]
    fn capture_reserves_tail_capacity_but_release_can_use_it() {
        let mut l = Limiter::new(Some(2), Some(4));
        assert!(l.allow_with_headroom(0, 0, 1));
        l.record(0);
        assert!(!l.allow_with_headroom(1, 1, 1));
        assert!(l.allow(1, 1));
        assert!(!l.allow_with_headroom(60_000, 3, 1));
        assert!(l.allow(60_000, 3));
    }
    #[test]
    fn unknown_quota_never_guesses_and_window_expires() {
        let mut l = Limiter::default();
        assert!(!l.allow(0, 0));
        l.minute = Some(1);
        l.daily = Some(3);
        assert!(l.allow(0, 0));
        l.record(0);
        assert!(!l.allow(59_999, 1));
        assert!(l.allow(60_000, 1));
        assert!(!l.allow(120_000, 3));
    }
}
