// ALGORITHM VERSION 1.2 — 2026-11-13
// 1. Treat a 32 ms frame above -40 dB RMS as speech; retain every sample regardless.
// 2. Pieces mode: wait for the 30/60/90 second minimum and 0.4 s quiet before a cut.
//    Live mode (live.rs): wait for a 3 s minimum turn and 0.7 s quiet, so Google
//    finalizes phrase by phrase and only the newest words stay provisional.
// 3. Never force a boundary in continuous speech; each sample belongs to one piece.
// 4. Unary requests need known minute/day caps; retain audio when capacity is missing.
// 5. Reserve one request during capture for release/retry; reset daily counts in Pacific time.
// A conservative energy gate replaces neural VAD so cloud mode loads no local model.
// It may delay a cut in noise; it must never discard or shorten the original recording.
// v1.0 — SOP-10742: model-free pause signal shared by capture and manual recovery.
// v1.2 — SOP-10847: Cutter::set_quiet_ms lets Live wait longer for an unfinished phrase.
// v1.1 — SOP-10774: Cutter takes its own minimum/quiet; Live uses 3 s / 0.7 s, pieces keep 30-90 s / 0.4 s.
pub fn is_speech(frame: &[f32]) -> bool {
    !frame.is_empty() && frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32 > 0.0001
}
// Sample-exact pause partitioning and conservative quota scheduling.
// Model-free energy scores run outside the capture thread.
use std::collections::VecDeque;

pub const RATE: usize = 16_000;
pub const FRAME: usize = 512;
/// Only detected pauses are eligible boundaries. A target is a minimum, never
/// a deadline: if speech continues we keep the audio instead of guessing a cut.
pub struct Cutter {
    since: usize,
    quiet: usize,
    target: usize,
    quiet_needed: usize,
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
            quiet_needed: RATE * 4 / 10,
        }
    }
    /// A short-turn cutter (Live phrases): `min_ms` of audio, then `quiet_ms`
    /// of detected non-speech. Same sample-exact, never-force-a-cut rules.
    pub fn with_millis(min_ms: u32, quiet_ms: u32) -> Self {
        Self {
            since: 0,
            quiet: 0,
            target: min_ms as usize * RATE / 1000,
            quiet_needed: (quiet_ms as usize * RATE / 1000).max(1),
        }
    }
    /// Change the quiet needed for the next cut (Live waits longer mid-sentence).
    pub fn set_quiet_ms(&mut self, quiet_ms: u32) {
        self.quiet_needed = (quiet_ms as usize * RATE / 1000).max(1);
    }
    /// Return the end of at least 0.4 seconds of detected non-speech, after
    /// the configured target. Every sample, including silence, stays in order.
    /// Detection can be wrong; never force a cut inside continuous speech.
    pub fn push(&mut self, frame: &[f32], speech: bool) -> Option<usize> {
        self.since += frame.len();
        self.quiet = if speech { 0 } else { self.quiet + frame.len() };
        if self.since >= self.target && self.quiet >= self.quiet_needed {
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
    fn live_phrase_cutter_cuts_after_short_turn_and_longer_quiet_only() {
        let mut c = Cutter::with_millis(3000, 700);
        // 2.9 s of speech is below the minimum turn, even with a long pause after.
        for _ in 0..90 {
            assert_eq!(c.push(&[0.1; FRAME], true), None);
        }
        for _ in 0..3 {
            assert_eq!(c.push(&[0.0; FRAME], false), None);
        }
        // Minimum reached, but 0.7 s (22 frames) of quiet is still required.
        let mut cut = None;
        let mut quiet_frames = 3;
        while cut.is_none() {
            quiet_frames += 1;
            cut = c.push(&[0.0; FRAME], false);
        }
        assert_eq!(quiet_frames, 22);
        assert_eq!(cut, Some((90 + 22) * FRAME));
        // Speech resets the quiet counter.
        let mut c = Cutter::with_millis(3000, 700);
        for _ in 0..100 {
            c.push(&[0.1; FRAME], true);
        }
        for _ in 0..21 {
            assert_eq!(c.push(&[0.0; FRAME], false), None);
        }
        assert_eq!(c.push(&[0.1; FRAME], true), None);
        assert_eq!(c.push(&[0.0; FRAME], false), None);
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
    fn continuous_speech_past_upload_limit_is_retained_and_rejected_without_mutation() {
        let original = vec![0.25; RATE * 300 + 3];
        for target in [30, 60, 90] {
            let mut cutter = Cutter::new(target);
            let mut retained = Vec::new();
            for frame in original.chunks(FRAME) {
                retained.extend_from_slice(frame);
                assert_eq!(cutter.push(frame, true), None);
            }
            assert_eq!(crate::encode_wav(&retained), Err(crate::Error::TooLong));
            assert_eq!(retained, original);
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

#[cfg(test)]
mod energy_tests {
    #[test]
    fn model_free_pause_signal_retains_quiet_and_speech_samples() {
        assert!(!super::is_speech(&[0.0; 512]));
        assert!(!super::is_speech(&[0.001; 512]));
        assert!(super::is_speech(&[0.1; 512]));
        let mut cutter = super::Cutter::new(30);
        let mut boundaries = 0;
        for _ in 0..1000 {
            boundaries += usize::from(
                cutter
                    .push(&[0.1; 512], super::is_speech(&[0.1; 512]))
                    .is_some(),
            );
        }
        assert_eq!(boundaries, 0);
        for _ in 0..13 {
            if let Some(n) = cutter.push(&[0.0; 512], false) {
                assert_eq!(n, 1013 * 512);
                boundaries += 1;
            }
        }
        assert_eq!(boundaries, 1);
    }
}
