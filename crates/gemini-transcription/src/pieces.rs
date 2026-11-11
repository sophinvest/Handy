//! Sample-exact pause partitioning and conservative quota scheduling. VAD scores
//! are supplied by the app's one-thread Silero worker, never the capture thread.
use std::collections::VecDeque;

pub const RATE: usize = 16_000;
pub const FRAME: usize = 512;
#[derive(Default)]
pub struct Cutter {
    since: usize,
    quiet: usize,
    quietest: Option<(f32, usize)>,
}
impl Cutter {
    /// Returns a relative cut offset. A cut is considered only after 20 seconds.
    /// Prefer 0.4 s of Silero non-speech; after 45 s use the quietest frame since
    /// 20 s. The caller retains the suffix: no overlap, dropped silence or gaps.
    pub fn push(&mut self, frame: &[f32], speech: bool) -> Option<usize> {
        self.since += frame.len();
        self.quiet = if speech { 0 } else { self.quiet + frame.len() };
        if self.since >= 20 * RATE {
            let energy = frame.iter().map(|s| s * s).sum::<f32>() / frame.len().max(1) as f32;
            if self.quietest.is_none_or(|(e, _)| energy < e) {
                self.quietest = Some((energy, self.since));
            }
            let cut = if self.quiet >= RATE * 4 / 10 {
                Some(self.since)
            } else if self.since >= 45 * RATE {
                self.quietest.map(|(_, offset)| offset)
            } else {
                None
            };
            if let Some(cut) = cut {
                self.since -= cut;
                self.quiet = 0;
                self.quietest = None;
                return Some(cut);
            }
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
        while self
            .sent
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= 60_000)
        {
            self.sent.pop_front();
        }
        self.minute
            .is_some_and(|n| n > 0 && self.sent.len() < n as usize)
            && self.daily.is_some_and(|n| n > day_count)
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
    fn no_pause_uses_quietest_point_after_minimum() {
        let mut c = Cutter::default();
        let mut cut = None;
        for i in 0..(45 * RATE).div_ceil(FRAME) {
            let quiet = i == 700;
            cut = c
                .push(&vec![if quiet { 0.01 } else { 1.0 }; FRAME], true)
                .or(cut);
        }
        assert_eq!(cut, Some(701 * FRAME));
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
