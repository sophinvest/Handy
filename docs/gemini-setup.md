# Handy Gemini for Ubuntu

This Sophinvest fork adds Google Gemini alongside Handy’s local models. It uses the same application identity and original Linux clipboard/paste implementation. The built-in updater is disabled so an official Handy update cannot silently remove Gemini. Sophinvest must merge upstream fixes and publish another fork release.

## Install

Quit Handy, download the `.deb` from [Sophinvest releases](https://github.com/sophinvest/Handy/releases), and install it with `sudo apt install ./Handy_0.10.0-gemini.6_amd64.deb`. Reopen Handy. Your settings and recordings remain in place. The unsigned package is published only in the Sophinvest repository. To roll back, quit and reinstall the official Handy package.

## Choose Google or a local model

**Models** shows Handy’s local catalog, downloaded models and the expandable **Google Gemini (cloud) — choose or configure** row. Google setup no longer occupies the entire Models screen. Expand the row to configure Google; collapse it to browse local models. Selecting a local model restores ordinary offline dictation. General and onboarding use the same Google controls.

1. Create your own key in [Google AI Studio](https://aistudio.google.com/apikey), using a dedicated project **without a linked billing account**. The app never changes billing; a key from a billed project is not made free-only by this app.
2. Paste the key into Handy’s Google settings and click **Verify model and save key**. ListModels discovers the unary and Live Transcribe resource names available to the key. No key needs to be sent to Sophinvest.
3. Click **Choose Google Gemini**. Local speech and language models unload. Google mode never runs a local recognition or neural pause-detection model, including if an old settings file still enables local fallback. For offline recognition, explicitly select a local model instead.
4. Keep **Live preview, then pause pieces if unavailable** selected. Speak into a text editor with Handy’s normal shortcut. The bottom overlay identifies the active source throughout recording and finalization.

The key is stored through Linux Secret Service/GNOME Keyring. If unavailable, Handy reports its private-file fallback: `~/.local/share/com.pais.handy/gemini-credentials/api-key` (or the corresponding `$XDG_DATA_HOME` path), file mode 0600 in a 0700 directory. It is never in settings JSON or diagnostic logs.

## Read the overlay

- **Google: connecting**: capture has started and audio is buffered while the socket connects. The bounded queue holds approximately 65 seconds; its overflow is an error and the independent full recording is saved.
- **● Google live**: Google accepted the Live setup. Audio frames are sent continuously and only Google transcript messages populate the preview. The label identifies the connected engine; logs distinguish outgoing audio and received transcript activity.
- **Google: pause pieces**: the selected mode uses pieces, Live is unavailable, or a Live connection failed. A one-line log explains the fallback. Pieces can take longer after release; this label does not promise Live latency.
- **Google: uploading**: Single upload or an explicit file/history upload. This mode intentionally sends the complete file after recording.
- **Google: error** or Handy’s error notice: the recording is retained. No partial hypothesis is pasted as a completed transcript.
- **Local model**: offline recognition was explicitly selected.

Live sends the audio-stream-end marker on release, then drains independently ordered transcript messages for up to 1.5 seconds. It returns earlier only when the socket ends after authoritative final text; a brief quiet interval or turn-complete marker cannot end the drain early. It does not switch to a whole-recording upload during a failed Live finalization. A timeout saves the clip for explicit retry instead of pasting an incomplete prefix. This bounded finalization targets a paste within about two seconds on a healthy connection; Google/network/hardware latency is not guaranteed.

No local neural VAD runs in Google mode. A small RMS energy calculation identifies quiet frames for pause pieces and long-session rollover. It keeps **all** audio, including quiet speech; it only decides where a piece ends. Background noise can delay a boundary. Live reconnects near nine minutes at a quiet boundary; audio arriving during reconnect remains buffered.

## Check streaming on the first laptop try

Start Handy with `--debug`, or use Handy’s log viewer. Never send the key or dictated content through an issue. Look for these metadata-only lines in order:

1. `gemini_capture local_model_loaded=false pause_detector=energy_only`
2. `gemini_live setup_complete model=...` and overlay **● Google live**
3. `gemini_live audio_out frames=... samples=...` while still speaking
4. `gemini_live partial_in count=... audio_frames=...` **before release**, followed by more outgoing frames
5. On release, `gemini_live audio_end` then `gemini_live finalized_after_drain release_ms=...` or `gemini_live finalized_on_close release_ms=...`

If it switches to pieces, find `gemini_live fallback=pause_pieces reason=...`, optionally preceded by `server_error code=...` or `socket_closed code=...`. The reason excludes server-supplied text and credentials. This is the evidence missing from the .4 report. Share only those metadata lines to diagnose the actual failure.

Use a 60-second and three-minute passage. Measure shortcut release to pasted text in the destination editor, record the overlay source and finalization duration, and check the leading words and sentence spaces. No Sophinvest test has yet measured Google recognition accuracy, real network latency, physical microphone continuity or laptop paste with Jean’s key. Local mock timings are transport evidence, not those results.

## Pieces, saved clips and quotas

Configure unary requests/minute and requests/day from [your project limits](https://aistudio.google.com/rate-limit). Unknown limits stop piece uploads with the clip retained. The rolling-minute and Pacific-day counts include retries and file/history calls; other clients also consume the project quota. The app never guesses a free allowance or retries a failed request automatically.

Pause targets are 30, 60 or 90 seconds. After the minimum target, a cut waits for 0.4 seconds of quiet. Continuous speech stays together, every sample belongs to one piece, and one request is reserved during recording for a final tail or retry. Unary uploads over five minutes fail with the recording preserved. Large/noisy uninterrupted clips may need manual subdivision.

On an error, **Ctrl+Shift+F8** explicitly retries one remaining piece of the oldest saved clip, preserving completed progress; it pastes only after all pieces finish. **Retry oldest clip for preview** retains the WAV. **Open saved clips** opens the recovery folder. Successful retry-and-paste removes only that pending clip. Ordinary history retention never deletes pending clips. Retry uses the same model-free pause signal and segment joining as dictation.

On Ubuntu, use `echo $XDG_SESSION_TYPE` to identify X11/Wayland. Keep the original Handy paste tools/permissions. Do not change session settings to obtain a screenshot. An optional AppImage needs FUSE and the host Vulkan loader; the `.deb` declares `libvulkan1`.

## Verification and maintenance

The transport suite exercises a complete mock WebSocket handshake, provisional and authoritative responses, finalization with/without turnComplete, final-after-turnComplete ordering (including several delayed finals with no interim packet), timeout preservation, segment spacing and real-time 60/180-second streams with delayed connection and exact audio counts. It uses no Google key. Native packaging and process evidence are listed with the release; do not infer a physical paste result from a socket test.

[Google Live Transcribe protocol](https://ai.google.dev/gemini-api/docs/live-api/live-transcribe) · [WebSocket reference](https://ai.google.dev/api/live) · [Pricing](https://ai.google.dev/gemini-api/docs/pricing/#gemini-3.5-transcribe-live) · [Maintenance](gemini-maintenance.md)

### Recording boundaries in gemini.6

The Live connection uses explicit start/stop signals matching the recording button. This prevents Google's automatic speech detector from ending a sentence at an internal pause. Provisional Google words remain underlined until Google's final text arrives. Release drains incoming finals for up to 1.5 seconds; an unfinished hypothesis is never pasted as final, and the original clip remains available on failure. This is streaming, not a full-recording upload after release.

For long recordings, Handy also finalizes a speech turn at a detected quiet pause after at least 30 seconds, using the same simple volume calculation as pause pieces. The connection remains Live, no samples are removed, and no local neural model runs. If continuous speech or noise supplies no pause and Google cannot finish in the short release window, Handy reports the failure and keeps the recording instead of pasting provisional text.
