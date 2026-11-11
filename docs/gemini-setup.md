# Handy Gemini preview

This is the Sophinvest fork of Handy. It installs separately as **Handy Gemini** and does not replace your existing Handy settings. Quit the original Handy before testing so two apps do not register the same dictation shortcut. This fork disables the upstream self-updater, which would otherwise replace the cloud engine with the original app.

## Install

Download from the Sophinvest fork's release page. Use the Windows x64 installer, the Apple Silicon DMG for an M-series Mac, or the Intel DMG for an Intel Mac. These preview builds are unsigned.

- **Windows:** run the installer. If Windows SmartScreen appears, select **More info → Run anyway** for the file downloaded from this release. Allow microphone access when prompted.
- **Mac:** open the DMG, drag Handy Gemini to Applications, and open it. If macOS blocks it, open **System Settings → Privacy & Security → Open Anyway**, then confirm. Grant Microphone and Accessibility access to Handy Gemini. The original Handy's grants do not apply to this separate app.

## Create the free-only key

1. Open [Google AI Studio API keys](https://aistudio.google.com/apikey) and sign in.
2. Choose **Create API key**. Use a new Google project dedicated to Handy, without a linked billing account. Do not choose an existing project with billing enabled.
3. Copy the API key and save your own backup in 1Password. Do not send it in Paperclip, chat, email, or a screenshot.
4. Leave billing unlinked. The app does not change Google project billing, and cannot independently certify the billing state of a key. A billed project does not become free-only because this app is selected.

## Set up Handy Gemini

1. Open Handy Gemini. Complete the language and permission steps. At model selection, open **Google Gemini (cloud)**. Returning users can use **Settings → General → Google Gemini (cloud)** or **Models → Google Gemini (cloud)**.
2. Paste the key into **Gemini API key** and click **Verify model and save key**. This performs Google's `ListModels` call and saves the key to Windows Credential Manager or macOS Keychain. It does not save the key in the settings JSON. The password field clears after use.
3. The verified model name appears. If Google does not list a compatible non-streaming Transcribe model, setup stops instead of guessing an ID or silently choosing another provider.
4. Click **Choose Google Gemini**. Keep **Allow local fallback** off. No local speech model, voice detector, language detector, or live decoding runs in this mode. The audio spectrum animation is bypassed to reduce CPU work.
5. Under **Settings → Advanced → Custom Words** (Handy's existing Custom Words control), add lender, street, and person names you commonly dictate. These are sent as vocabulary hints with each clip.
6. Click into a text box outside Handy. Use the normal dictation shortcut. Speak, then release the key. The complete 16 kHz mono recording is sent once after release, and the returned transcript uses Handy's normal paste path. No streaming upload is used.

## When Google is unavailable

A quota error displays **Gemini free limit reached**. Offline and timeout errors are also reported. The app makes one request, with a 10-second network timeout, and preserves the audio in its `gemini-pending` folder. There is no automatic network retry. Clips over five minutes are preserved without uploading.

- To retry and paste at the cursor: click the destination text box and press **Ctrl+Shift+F8**. This retries the oldest saved clip once. Change this shortcut in the Google settings section if it conflicts with another app.
- To review before pasting: open Google settings, click **Retry oldest clip for preview**, then **Copy transcript** and paste into your destination. Preview leaves the recording saved.
- **Open saved clips** opens the recovery folder. You can keep, split, or delete those WAV files yourself. These pending files survive restart and ordinary history cleanup. Previewing/copying does not remove them; a successful retry-and-paste removes exactly that pending clip.
- A local fallback model is optional. Download one through Handy's normal Models page, select it under **Local fallback model**, and enable the checkbox. It loads only after a cloud failure, then unloads after that clip. This uses the laptop's CPU and is intentionally off by default.

To switch fully back to an offline model, choose any downloaded local model from the usual model picker. Its original transcription path remains in place.

## Validation and remaining laptop checks

Automated transport tests use a local mock server and fixture credentials, never a Google key. They cover model listing, WAV/PCM encoding, vocabulary, verbatim mode, structured/plain text responses, quota/auth/server errors, offline connection failure, timeout, and empty/truncated responses. The transport's sixty-second structural buffer test compares every sample, including silence. A separate real `CaptureSink` test checks 60 seconds and a final partial frame without invoking VAD or the spectrum FFT.

**Not established by mocks:** actual Google accuracy, real network latency, OS keychain behavior on Jean's laptop, microphone continuity under thermal throttling, or paste permissions. The 2–3 second release-to-paste target has not been measured against Google. A mock response is not a speech recognition accuracy test.

After installing, try a 10–20 second passage with your names and measure release-to-paste time. Then dictate a known 60-second passage while the laptop is busy, compare every word with the reference, and confirm the complete text eventually pastes. A separate capture thread removes inference-related stalls; no desktop app can guarantee gap-free audio if the operating system stops scheduling the capture thread or the audio driver drops samples. We cannot certify a failing fan or thermal-throttling scenario from this server.

Optional developer stress harness: `python scripts/gemini-cpu-stress.py --seconds 60` saturates the available CPU cores for a bounded interval while the user dictates. It stops all of its own workers on timeout or Ctrl+C. It does not access the microphone, Google key, or clipboard. Only run it as an intentional test on the laptop being evaluated.

## API reference

[Google transcription configuration](https://ai.google.dev/gemini-api/docs/generate-content/transcribe) · [Google ListModels](https://ai.google.dev/api/models)
