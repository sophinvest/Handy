# Maintaining the Handy Gemini fork

The starting upstream commit is `f248301` on Handy main, reporting app version 0.10.0. The independent Gemini transport and tests live in `crates/gemini-transcription/`; Tauri integration lives in `src-tauri/src/gemini.rs`; shared settings UI lives in `src/components/settings/GeminiSettings.tsx`. No clipboard, keyboard implementation, Linux paste, or upstream engine-supervisor file was changed.

## Update policy

The built-in updater is disabled by the default `gemini-fork` build feature and has no upstream download endpoint. It is not redirected to an unsigned update feed. Publish reviewed Linux packages manually after bringing in an official Handy release. Do not enable the original updater as a shortcut: that would remove the cloud engine.

## Upstream files changed

The following is the concrete integration surface, rather than a claim that this is a loadable plugin. Language files carry additive English fallback strings, not rewrites of existing translations. Workflow guards prevent the upstream's signed multi-platform jobs from running in the fork.

- [`.github/workflows/build-test.yml`](../.github/workflows/build-test.yml)
- [`.github/workflows/code-quality.yml`](../.github/workflows/code-quality.yml)
- [`.github/workflows/main-build.yml`](../.github/workflows/main-build.yml)
- [`.github/workflows/nix-check.yml`](../.github/workflows/nix-check.yml)
- [`.github/workflows/playwright.yml`](../.github/workflows/playwright.yml)
- [`.github/workflows/pr-test-build.yml`](../.github/workflows/pr-test-build.yml)
- [`.github/workflows/test.yml`](../.github/workflows/test.yml)
- [`.gitignore`](../.gitignore)
- [`package.json`](../package.json)
- [`src-tauri/Cargo.lock`](../src-tauri/Cargo.lock)
- [`src-tauri/Cargo.toml`](../src-tauri/Cargo.toml)
- [`src-tauri/src/actions.rs`](../src-tauri/src/actions.rs)
- [`src-tauri/src/audio_toolkit/audio/sink.rs`](../src-tauri/src/audio_toolkit/audio/sink.rs)
- [`src-tauri/src/audio_toolkit/audio/sink/tests.rs`](../src-tauri/src/audio_toolkit/audio/sink/tests.rs)
- [`src-tauri/src/audio_toolkit/vad/silero.rs`](../src-tauri/src/audio_toolkit/vad/silero.rs)
- [`src-tauri/src/commands/models.rs`](../src-tauri/src/commands/models.rs)
- [`src-tauri/src/lib.rs`](../src-tauri/src/lib.rs)
- [`src-tauri/src/managers/audio.rs`](../src-tauri/src/managers/audio.rs)
- [`src-tauri/src/managers/model.rs`](../src-tauri/src/managers/model.rs)
- [`src-tauri/src/managers/transcription.rs`](../src-tauri/src/managers/transcription.rs)
- [`src-tauri/src/managers/transcription/job.rs`](../src-tauri/src/managers/transcription/job.rs)
- [`src-tauri/src/settings.rs`](../src-tauri/src/settings.rs)
- [`src-tauri/tauri.conf.json`](../src-tauri/tauri.conf.json)
- [`src/bindings.ts`](../src/bindings.ts)
- [`src/components/onboarding/OnboardingFlow.tsx`](../src/components/onboarding/OnboardingFlow.tsx)
- [`src/components/onboarding/OnboardingShell.tsx`](../src/components/onboarding/OnboardingShell.tsx)
- [`src/components/onboarding/StandaloneScreens.tsx`](../src/components/onboarding/StandaloneScreens.tsx)
- [`src/components/onboarding/screens/Model.tsx`](../src/components/onboarding/screens/Model.tsx)
- [`src/components/settings/general/GeneralSettings.tsx`](../src/components/settings/general/GeneralSettings.tsx)
- [`src/components/settings/models/ModelsSettings.tsx`](../src/components/settings/models/ModelsSettings.tsx)
- [`src/components/settings/models/modelEntries.ts`](../src/components/settings/models/modelEntries.ts)
- [`src/i18n/locales/ar/translation.json`](../src/i18n/locales/ar/translation.json)
- [`src/i18n/locales/bg/translation.json`](../src/i18n/locales/bg/translation.json)
- [`src/i18n/locales/ca/translation.json`](../src/i18n/locales/ca/translation.json)
- [`src/i18n/locales/cs/translation.json`](../src/i18n/locales/cs/translation.json)
- [`src/i18n/locales/da/translation.json`](../src/i18n/locales/da/translation.json)
- [`src/i18n/locales/de/translation.json`](../src/i18n/locales/de/translation.json)
- [`src/i18n/locales/en/translation.json`](../src/i18n/locales/en/translation.json)
- [`src/i18n/locales/es/translation.json`](../src/i18n/locales/es/translation.json)
- [`src/i18n/locales/fr/translation.json`](../src/i18n/locales/fr/translation.json)
- [`src/i18n/locales/he/translation.json`](../src/i18n/locales/he/translation.json)
- [`src/i18n/locales/hi/translation.json`](../src/i18n/locales/hi/translation.json)
- [`src/i18n/locales/id/translation.json`](../src/i18n/locales/id/translation.json)
- [`src/i18n/locales/it/translation.json`](../src/i18n/locales/it/translation.json)
- [`src/i18n/locales/ja/translation.json`](../src/i18n/locales/ja/translation.json)
- [`src/i18n/locales/ko/translation.json`](../src/i18n/locales/ko/translation.json)
- [`src/i18n/locales/ne/translation.json`](../src/i18n/locales/ne/translation.json)
- [`src/i18n/locales/nl/translation.json`](../src/i18n/locales/nl/translation.json)
- [`src/i18n/locales/pl/translation.json`](../src/i18n/locales/pl/translation.json)
- [`src/i18n/locales/pt/translation.json`](../src/i18n/locales/pt/translation.json)
- [`src/i18n/locales/ru/translation.json`](../src/i18n/locales/ru/translation.json)
- [`src/i18n/locales/sv/translation.json`](../src/i18n/locales/sv/translation.json)
- [`src/i18n/locales/tr/translation.json`](../src/i18n/locales/tr/translation.json)
- [`src/i18n/locales/uk/translation.json`](../src/i18n/locales/uk/translation.json)
- [`src/i18n/locales/vi/translation.json`](../src/i18n/locales/vi/translation.json)
- [`src/i18n/locales/zh-TW/translation.json`](../src/i18n/locales/zh-TW/translation.json)
- [`src/i18n/locales/zh/translation.json`](../src/i18n/locales/zh/translation.json)

`.github/workflows/build.yml` also adds the Linux D-Bus development dependency for Secret Service. The shared Debian configuration declares `libvulkan1` because the shipped, dynamically loaded Vulkan backend needs it even though the main executable's dependency scan does not find it. AppImage deliberately uses the host Vulkan loader; its setup instructions cover that dependency. New files are the isolated Gemini modules/tests, shared cloud settings component, bounded CPU-load harness and these guides.

## Bring in the next official Handy release

1. Start from a clean checkout of `sophinvest/Handy`, with all fork work committed and pushed. Add the read-only upstream remote if needed: `git remote add upstream https://github.com/cjpais/Handy.git`.
2. Fetch both remotes and upstream tags: `git fetch origin` and `git fetch upstream --tags`.
3. Create a local integration branch from `origin/main`, then merge the selected official release tag into it, for example `git merge --no-ff v0.10.1`. Use the actual release tag. A merge preserves already published fork history; do not force-rebase published main. For an unpublished integration branch, `git rebase <official-tag>` is also possible, but review the same file list above at each conflict.
4. Resolve conflicts around model selection, recording start/stop, the common transcription completion path, default settings and generated Tauri types. Keep the cloud branch before every local model/VAD/live-decode path. Keep fallback off and preserve saved clips. Keep the original paste code from the new Handy release.
5. Regenerate bindings by Handy's normal debug-build export path if its command types changed. Keep key material out of settings types and generated bindings. Keep the Gemini key in Secret Service or the explicit 0600 Linux fallback.
6. Bump the fork package version consistently in `package.json`, `src-tauri/Cargo.toml`/lockfile and `src-tauri/tauri.conf.json`. Keep product name `Handy` and app identifier `com.pais.handy` so Ubuntu upgrades reuse the existing configuration.
7. Run targeted Gemini transport, storage and capture tests while resolving conflicts. Once stable, run the full frontend/native gates once and build the Ubuntu `.deb` and AppImage using the existing reusable build workflow. Check the installer contents and actual X11/Wayland paste on Linux. The real Google-key/CPU-stress test must be reported separately from mocks.
8. Push to the Sophinvest fork only, publish packages only for the tested current commit, and update this changed-file inventory. Never send a PR, issue or comment upstream without separate board authorization.

The transport contains no hard-coded model selection: setup validates the key using ListModels, and dictation uses that verified returned resource name. Future Google API changes are confined to the transport and model-discovery tests.

## Live-source integration

Version 0.10.0-gemini.3 adds `crates/gemini-transcription/src/live.rs` (WebSocket protocol), `pieces.rs` (pause boundaries and quota window), and `src-tauri/src/gemini_stream.rs` (one network/VAD worker and recovery progress). It reuses the existing StreamTextEvent overlay renderer; no new paste implementation or upstream streaming engine is added. Existing integration files in the inventory gain small branches for recording callback selection, cloud finalization/cancellation, key discovery, configuration and shared settings. The file-transcription job explicitly bypasses the active recording session. Updater behavior stays disabled.

On an upstream merge, check that `CaptureSink` still calls the cloud callback with every valid sample and that its callback is non-blocking. Verify cloud cancellation closes the channel, finish drains all delivered samples, and only recording completion consumes a cloud session. Preserve the private recovery offset/prefix and the rate limiter across retries. Keep unary upload size bounds independent from the maximum saved-recording duration. Never silently commit a speculative interim transcript.

Native review for the live Ubuntu release transfers one exclusive request guard from session creation to its worker and back to finalization. Cancelling does not unlock a still-running request, and a failed recording start returns its borrowed detector. Cloud samples are never padded after capture; final sample-count validation compares the original recording exactly. Live rollover failures replay the complete buffered recording through pause pieces, preserving unary progress for recovery. The cloud Live overlay remains selected through finalization and respects the disabled-overlay setting. These changes stay within the existing integration files above.

The expanded cloud setup scrolls inside the model onboarding screen above its fixed footer, in both recommended-model and all-model views. Other onboarding screens keep their existing layout. The shared description reflects Live transport and worker pause detection across General, Models and onboarding in all locale fallback strings.

## Accuracy controls (.4)

The shared cloud settings now persist a 30/60/90-second minimum pause target, default 60 seconds. `pieces.rs` cuts only after a detected 0.4-second pause; it no longer guesses an energy minimum as a forced speech boundary. The worker and manual recovery use the same setting. During capture the quota window retains capacity for one final-tail request; release and explicit recovery can consume that capacity. No overlap/text-deduplication heuristic is shipped. Audio remains authoritative and over-size segments fail visibly with the WAV retained.

Coverage: General, Models, initial and replacement onboarding share `GeminiSettings`; mode save/status, native settings defaults, generated command/types, the streaming worker and saved-clip recovery all carry the target. File/history transcription deliberately stays single-upload because it is not dictation. The comparison worksheet is `docs/gemini-comparison.md`; its real-key results remain blank until measured locally. Keep these contracts when integrating upstream updates.

The Ubuntu .4 native review also checks legacy settings default to 60 seconds, all supported targets round-trip, invalid targets/zero known caps are rejected, and uninterrupted speech beyond the unary upload limit remains sample-exact and fails encoding without mutation. The shared command uses the same validation helper exercised by its native regression test. Reuse the .3 callback-buffer and Silero CPU evidence when those implementations are unchanged; these synthetic measurements do not establish Google accuracy or physical microphone continuity.

The shared cloud form refreshes background status on focus and clip events without resetting unsaved mode, target or quota edits. Linux native select popups can return window focus before Save is clicked; treating that event as a full form reload discards the chosen target. Initial load and explicit completed actions still synchronize the form from native settings. Check actual mouse selection and Save/reload in General, Models and onboarding after future UI updates.

## .5 corrective release

The Google mode path must not load any local model, including Silero VAD and a legacy opt-in fallback. `gemini_stream.rs` and saved-clip retry now use a shared model-free energy signal in `pieces.rs`; `transcription.rs` rejects local loads while Gemini is selected. Keep these guards during an upstream merge. All dictated samples remain authoritative in CaptureSink’s independent buffer.

`live.rs` performs the same complete setup in production and mock tests. It logs counters and safe numeric failure codes, keeps secrets/text out of diagnostics, joins authoritative utterances with separators, and bounds release finalization. Preserve transcript messages arriving independently of turnComplete: drain the full bounded window rather than returning after a quiet gap. Several late finals can arrive without an interim packet; the regression covers this ordering. An ended socket after authoritative final text can finish earlier. Never restore a silent release-time whole-file fallback. The overlay’s source indicator is translated and follows actual backend state; it must remain visible during finalization too.

Additional upstream surfaces changed by .5: `src/overlay/RecordingOverlay.tsx` and `.css`. Models uses compact expandable Google setup, shows the full upstream local list, and no longer hides uncurated choices behind a text search. Local models remain handled by Handy’s existing table/picker/tray. File/history deliberately remains unary upload.

The pause/limit algorithm now carries its first English narration at the top of `pieces.rs`. Before .5 there was no narration block. Version 1.0 documents the energy signal, minimum/quiet boundary, exact partition and quota headroom; future rule changes must bump it.

Provisional overlay text uses an underline on `.stext-cap .tentative`; committed text remains plain. A final text event moves the same phrase into the plain committed span. Local event fixtures are styling proof only and must never be presented as Google recognition evidence.

## Real Live finalization correction (gemini.6)

Real Google verification of .5 reproduced a release-time timeout when all authoritative input transcripts arrived during capture. Transcribe Live accepted the socket and sent interim/final packets, but supplied no additional final or turnComplete after audioStreamEnd. Do not require a new final after release. Drain the existing 1,500 ms window, then accept nonempty authoritative text if no interim remains. An ended socket can also return already-authoritative text. A pending interim still fails with the clip retained. The delayed 300/600 ms final regression remains required. No local model, unary backfill, larger timeout or clipboard change is involved.

The ordinary model-list endpoint can omit the dedicated Live model even when a direct setup succeeds. Verify Live access using the actual setup path; do not infer unavailability from the list alone.

A second real-service control reproduced premature automatic endpointing: the public 11-second JFK fixture yielded only its first half under automatic activity detection, both at 32 ms and documented 100 ms chunk sizes. The documented manual activity mode returned the complete sentence. Therefore Live setup disables server automatic activity detection, sends activityStart once after setup, streams every sample, and sends activityEnd on release. No local detector decides the Live turn. Central socket fixtures assert these start/end signals as well as audio continuity. Reference: https://ai.google.dev/gemini-api/docs/live-api/live-transcribe#manual-vad

The first unsegmented manual three-minute real stream exceeded the 1.5-second finalization limit and was rejected without promoting provisional text. Live therefore reuses pieces.rs’s narrated Cutter (30-second minimum, 0.4-second quiet) to send activityEnd/activityStart at natural pauses during capture. NO_INTERRUPTION prevents the new activity from cancelling pending final text. Audio remains on the same socket, every sample is sent once, and no whole-file request is issued. Sustained speech/noise with no qualifying pause may still exceed the service’s finalization time; retain the clip and report failure instead of silently pasting a partial.

Real testing also showed that sending the next activityStart before the preceding authoritative final can make Google ignore the next audio. At a pause boundary, capture therefore queues every subsequent raw frame while the prior turn finalizes (bounded to 1.5 seconds), then sends activityStart and flushes those frames in order. Releasing during that wait shares the same overall 1.5-second release deadline; it never pastes the preceding turn while queued audio or an unfinished hypothesis remains. A delayed-final socket regression verifies every sample across this wait, including release before the boundary final arrives.

### Gemini .7 saved-clip recovery

`src-tauri/src/saved_clips.rs` owns immutable timestamp identities, numeric newest-first ordering, exact deletion, and the recognition → checked delivery → removal transaction. `gemini.rs` shares it between the native settings command (explicit ID for a row, native newest selection for the main action) and the keyboard action (latest ID selected under the cloud request guard). General, Models and both onboarding screens share `SavedClips` through `GeminiSettings`. No separate tray or CLI retry exists. File/history transcription is unrelated and unchanged. Translation keys and shortcut help remain consistent in every supported locale; the Google preview copy remains English where no translation exists.

Focused native regressions cover numeric ordering, older selection/deletion, single successful removal, stale/invalid IDs, empty queue and audio/progress retention on recognition/empty-text/delivery failure. `tests/saved-clips.spec.ts` mounts the real shared component with local mock IPC; it verifies row metadata, exact commands, duplicate-click suppression, errors and empty state. Its fixture entry is not included in the production bundle. Packaged tests use local WAVs and already-complete progress sidecars to reach the real delivery code without a Google request. Live transport, pause cutting, finalization, engine indicator, Google-only inference guards, free-only quotas and updater controls are unchanged from .6.

## Live phrase cadence (gemini.8, SOP-10774)

Real use of .7 showed the whole 30-second Live turn staying underlined, then flipping plain at once. Because the Live session uses manual activity signals, Google only emits `inputTranscription` finals after our `activityEnd`, and `live.rs` cut with the pieces defaults (30 s minimum, 0.4 s quiet). `Cutter::with_millis` now lets Live use its own cadence: `LIVE_MIN_TURN_MS = 3000` and `LIVE_QUIET_MS = 700` in `live.rs`. Pieces mode keeps the 30/60/90 s setting unchanged. All guarantees stay: every sample is sent once, audio is queued during the boundary wait (1.5 s bound shared with release), no local model. Narrated algorithm versions: `pieces.rs` 1.1, `live.rs` 1.1. Tune the two constants against real audio; a shorter quiet gives Google less context per turn and uses more Live turns, but Live is a single socket session, not one request per turn.

## Failure classes, retries and diagnostics (gemini.9, SOP-10790)

`crates/gemini-transcription/src/resilience.rs` (narrated, algorithm 1.0) owns the failure classes (`Error` gained `Rejected` and `Closed`), the retry policy and the `gemini_diag` log line. `live.rs` is algorithm 1.2: it records a resume point (samples and committed text covered by the last final) so `Live::resume` can reconnect and replay only the unfinished tail, paced at about 5x real time because real Google finals fall behind an unpaced replay (measured with an 11 s clip). `gemini_stream.rs` applies the policy to the Live connect, mid-session errors, release-time finalization, pieces and single uploads; `gemini.rs` applies it to saved-clip retry and exposes `gemini_diagnostics`.

Policy: transient (network, timeout, closed, 5xx) 2 retries after 0.4 s and 1.2 s; quota 1 retry after Google's `retryDelay` plus 1 s when that is at most 45 s; auth, rejected, empty and invalid audio never retry. Failure detail travels in a thread-local (`resilience::note`/`last`) so existing `Result<_, Error>` signatures did not change; read it on the same thread that made the call. A failed retry never switches to a local model, and the saved clip is untouched. Release-to-paste on the happy path is unchanged (finalization still waits the 1.5 s window).

The overlay engine state `google_retry|class|attempt|max|wait_seconds` is formatted by `engineLabel` in `RecordingOverlay.tsx`; new strings are in every locale file as English fallbacks. Log lines go through the normal Handy log (`handy.log`, 500 KB rotation, keep one old file); the file level must be Info or lower (the default is Debug).

## Silence no longer drops Live (gemini.10, SOP-10793)

Cause (from code; Jean's report): after a phrase's final, the next Live turn held only silence. Once it reached 3 s plus 0.7 s of quiet, `live.rs` sent `activityEnd`, but Google emits no final for a turn with no speech, so the 1.5 s wait timed out. The timeout was treated as a Live failure and the app fell back to pause pieces, whose engine label ("Google: pause pieces") also cleared the overlay text, so the box shrank and stayed small. Fix (`live.rs` algorithm 1.3): a turn with no detected speech is never ended; it stays open and the silence is sent as audio. `gemini_stream.rs` also keeps the text already on screen if a fallback to pieces ever does happen, until the replayed pieces cover it. The overlay no longer collapses during a pause.

## What can change the pasted text, label and status light (gemini.10, SOP-10793)

Reported: pasted text had different punctuation and sentence breaks than the final plain overlay text. Findings from code: the Live path returns exactly `Live.text.committed` (the same string the overlay shows) and `gemini::transcribe` uses it as-is, with no release-time Google request and no `outputTranscription`. Only two optional steps can rewrite the text afterwards: the LLM post-process (the post-process shortcut, `actions.rs`) and the user transcription hook (`hooks/transcription`). Not provable from here which one ran on Jean's laptop, so gemini.10 (a) writes one `paste_text llm_post_process=… hook=… rewritten_by_llm=…` line per paste (no text), and (b) switches the status line to "Post-processing…" for exactly the period either step runs (before, the working label kept saying "Google live" because it ignored the work kind).

Status light (`RecordingOverlay.tsx`, `engineStatus`): green = Google live connected; amber = connecting, retrying, pause pieces, uploading; red = Google error; unchanged accent for local models; grey while the microphone is still arming.

Live phrase boundary: quiet needed to end a phrase is now 1.5 s (was 0.7 s, `LIVE_QUIET_MS`, live.rs algorithm 1.4) so a thinking pause mid-sentence no longer splits it. This is a reasoned default, not a measured winner: scoring 0.7/1.2/1.5/2 s needs recorded real speech with thinking pauses and Jean's key.

## Google timeouts, empty turns and the detailed log (gemini.11, SOP-10811)

**What the "Google timeout" was (reproduced with Jean's recordings, real Google, real-time pace).** Google answers our `activityEnd` with `voiceActivity ACTIVITY_END`. When Google heard no speech in that turn it sends nothing else: no transcript, no `turnComplete`. Our own volume gate is cruder than Google's, so a breath, a click or a few quiet syllables counted as speech. `live.rs` then waited a fixed 1.5 s for a final that never comes, reported `no final transcript within 1.5 s` (shown as "Google timeout"), reconnected, replayed the same audio, hit the same wall three times and fell back to pause pieces. Measured: the 1.3 s and 2.4 s recordings (ff33b3e8, 3bf8ca48) and the first 3 s of the 63 s recording (b14f4969) all got only `ACTIVITY_END`, nothing else, for as long as we waited (25 s). Real finals arrive 0.3 to 0.6 s after `activityEnd` (never above 0.6 s in the 148 s and 63 s recordings), so a slow Google reply was not the cause. A second, smaller cause: sends had a 300 ms write limit, so a brief network stall counted as a timeout. It is now 5 s.

**Changes (live.rs algorithm 1.5, resilience.rs, gemini_stream.rs):** (1) an empty turn (ACTIVITY_END, then 3 s without a transcript) is skipped, not failed; on release it ends the dictation normally (or "no speech heard" if nothing at all was said). (2) the wait for a final fails only if Google is silent for 20 s (any message restarts the clock) or 60 s pass in total. (3) release settles on `generationComplete` (0.7 s quiet) or 1.5 s quiet instead of always waiting 1.5 s. (4) a Live session that drops mid-dictation is reconnected for about a minute (12 attempts, `LIVE_BACKOFF`, replaying only the unfinalized tail) before Live is given up; quota, key and rejected-request errors never fall back to pause pieces (they end the dictation with the clip saved); only network or server trouble that outlasts the reconnect window can still switch to pause pieces.

**The free-tier error.** Live is one socket per dictation and sends audio, not requests, so it does not count toward the roughly 3 requests per minute unary limit. Pause pieces send one unary request per chunk, plus retries. The "free-tier limit reached" came after the fall back to pieces. Confirmed in the same session: unary requests on Jean's key hit HTTP 429 after two calls, while Live streamed 148 s without any quota error. With the fall back removed for timeouts, the error cannot follow a timeout any more. If Live itself ever returns a quota error it shows as "Google: free-tier limit reached", with Google's own message in the log.

**Overlay.** The far-left dot is gone. The status dot sits right in front of the word "Google" (`.sstat` in `RecordingOverlay.css`): green = connected live, amber = connecting / retrying / pieces / uploading, red = error. The failed-transcription box now shows the specific error (the notice detail) below the transcript, and in the compact pill after the title.

**Detailed Google log.** Settings, Google section: tick "Detailed Google log" (off by default). It writes `google-trace-YYYY-MM-DD.log` into Handy's log folder (Linux: `~/.local/share/com.pais.handy/logs/`), one file per day, 50 MB cap, files older than 7 days deleted when it starts. "Copy debug log" copies the last 2 MB of today's file; "Open log folder" opens the folder. The switch is a flag file `google-trace-enabled` in the app data folder, so it survives restarts. Each line is `date time [tag] message`: `key` press/release, `mic` ready, `ws` connect/setup, `ws_out` audio sent (counts and bytes, no audio), `ws_in` every server message (kinds, size, time since the end of turn, transcript text), `ws_close` code and reason, `timer` when a wait fires, `turn` end/start/final/empty, `engine` state changes and the reason for any switch, `retry`, `diag`, `failure`, `overlay` show/hide, `overlay_text` committed and tentative text, `ui:overlay_label` the engine label and dot colour, `ui:overlay_notice` the error text shown, and `paste` text, result and timing. The key and audio bytes are never written; transcript text is, and the toggle says so. Code: `crates/gemini-transcription/src/trace.rs`, narrated, and the `gemini_trace_*` commands in `src-tauri/src/gemini.rs`.

**Protocol harness.** `scripts/gemini-live-sim.py WAV KEYFILE` streams a recording at real-time pace with the same turn rules and prints each final's delay after `activityEnd`. `--min-ms 99999999` sends the whole clip as one turn (reference transcript); `--quiet-ms 1200|1500|2000` compares phrase-end quiet. It reads the key from a file and never prints it.

**1.5 s phrase end, measured.** Reference = the same clip sent as one turn. 63 s clip (87 words): 1.2 s gave 3 word errors; 1.5 s gave 1 word error and one stray full stop mid-sentence ("Handy. software"); 2.0 s gave 2 word errors, no stray stop. 148 s clip (298 words): 1.2 s gave 4 word errors; 1.5 s gave 1; 2.0 s gave 1 but joined sentences (9 full stops against 14 in the reference). 1.5 s stays: it has the fewest word errors overall and keeps sentence breaks; 2.0 s reads well but loses sentence breaks on long dictation.
