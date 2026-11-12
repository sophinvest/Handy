# Compare recognition on the same recording

**Status: all Google and physical-laptop measurements below are unmeasured.** This worksheet is for Jean's Ubuntu laptop with his own no-billing key entered in Handy. Do not give the key to Sophinvest or put it in a shell command. The .4 source adds the required controls; use a published .4 installer after native verification, not the earlier .3 installer.

## Prepare once

1. Keep an exact reference passage and record it once as a 3–5 minute WAV (16 kHz mono is preferred). Include lender/street/person names, spoken amounts and dates, long sentences, short pauses, and intentional repetitions such as “very very” or a repeated number. Transcribe what was actually spoken in the reference, including repetitions. Save the original WAV unchanged.
2. Enter the no-billing key locally in Handy, verify returned unary/Live models, and configure the actual model/project limits shown in AI Studio. Use one project with no other clients for this test where possible. Record the model resource names, app version, custom words, project caps and remaining quota. Do not invent a free Live session/minute allowance. If unavailable, record “unknown.”
3. Route WAV playback to a virtual audio input, then select that input as Handy's microphone. On Ubuntu with PulseAudio or its PipeWire compatibility service, a temporary null sink and its monitor can provide this route: create it with `pactl load-module module-null-sink sink_name=handy_comparison` and keep the returned module number. Choose its monitor as the input using your sound routing tool, such as pavucontrol, and play the WAV with `paplay --device=handy_comparison reference.wav`. The default speaker/microphone should remain unchanged. Verify the selected monitor receives only the file; never use speaker-to-microphone replay for the same-audio comparison. If that virtual route is unavailable, record the comparison as not run rather than substitute five different dictations.
4. Use a text editor as the paste destination and enable Handy's overlay. Use toggle-recording if that makes it easier to keep recording across the whole playback. Start recording, wait the same short lead-in, start WAV playback, and stop recording immediately after the file ends. Retain the silent lead-in consistently across all runs. Record actual audio start, first visible preview, release, final combined text and paste times with a stopwatch or local screen recording. State the timing resolution; manual timing is approximate.

## Five sequential runs

For each row, choose its mode/target in the shared Google settings and click Save mode and limits **before recording**. Keep custom words, WAV, model resources, connection and destination identical. Complete one run and record the results before starting another. Wait for available project quota between runs; do not bypass caps or enable billing. A failed/deferred run is a result, not permission for an automatic retry loop.

| Run | Mode          | Pause target  | First preview | Release to final text | Release to paste | Requests / usage | Word errors / WER |
| --- | ------------- | ------------- | ------------- | --------------------- | ---------------- | ---------------- | ----------------- |
| 1   | Live preview  | 60 s fallback | unmeasured    | unmeasured            | unmeasured       | unmeasured       | unmeasured        |
| 2   | Pause pieces  | 30 s          | unmeasured    | unmeasured            | unmeasured       | unmeasured       | unmeasured        |
| 3   | Pause pieces  | 60 s          | unmeasured    | unmeasured            | unmeasured       | unmeasured       | unmeasured        |
| 4   | Pause pieces  | 90 s          | unmeasured    | unmeasured            | unmeasured       | unmeasured       | unmeasured        |
| 5   | Single upload | n/a           | n/a           | unmeasured            | unmeasured       | unmeasured       | unmeasured        |

Live packets remain one recognition stream. Confirm the Live resource was available; if Live failed and used pause pieces, label that run “fallback,” not “Live.” Handy's local daily counter counts unary requests only, including retries; record its before/after values and AI Studio usage separately. Live session/audio/token usage must come from the project's actual reported units, with any reporting delay noted. The app does not measure the project's shared quota consumption itself.

For each transcript:

- Save the final pasted text. Compare it with the exact spoken reference. Ignore punctuation/capitalization consistently, but keep number/name spellings and repetitions visible for separate review. Count substitutions (S), deletions (D), insertions (I), and reference words (N). Report `WER = (S + D + I) / N`, with the normalization rules, not just a percentage without counts.
- List every name and number error explicitly, with the reference and recognized text. Inspect words before/after each finalized segment or pause boundary for omissions and duplicates. Check intentional repeated words survive. No automatic text deduplication should remove them.
- Note pauses longer than the target, quota merging, saved recovery clips and manual retries. Time until final text and paste includes recovery if one was needed; do not present a successful-prefix preview as a completed transcript.
- A longer target is preferred only if the measured accuracy gain justifies slower previews. Record the chosen tradeoff and retain all transcripts. None of these target sizes is prescribed by Google.

Google supports [Live interim/final transcription and activity boundaries](https://ai.google.dev/gemini-api/docs/live-api/live-transcribe). If automatic end-of-speech produces premature final segments, compare Google's adjustable silence detection or manual activity boundaries in a separate local configuration experiment before changing the app. This candidate uses the default Live activity behavior; it exposes no untested silence knob. Final text is not guaranteed to remain revisable indefinitely. Long sessions reconnect only at detected pauses; failed rollover preserves/reprocesses the original recording rather than using an untested overlap reconciliation.

Unload the temporary audio route with `pactl unload-module <the returned module number>` and restore Handy's actual microphone before the hardware test. Remove only the module you created.

## Physical capture under CPU load

Use the real microphone and a known 60-second passage. Run the bounded `scripts/gemini-cpu-stress.py --seconds 60` (or `stress-ng --cpu 0 --timeout 60s`) on the laptop while dictating. Compare every pasted word, including repetitions, against the spoken reference and report elapsed release-to-paste time. Keep the audio and report any missing words, driver errors or delayed capture. Measure process CPU separately if possible. Stop if the laptop overheats; this test does not repair a failed fan.

The synthetic native buffer tests preserve supplied samples under CPU load, but cannot prove a physical driver delivered all microphone samples. No zero-word-loss claim can be made from those tests. Secret Service storage, X11/Wayland paste and real recognition also remain laptop checks.
