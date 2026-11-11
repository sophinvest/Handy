# Handy Gemini for Ubuntu

This is a Sophinvest fork of Handy, not a loadable plugin. It replaces the existing Handy application, uses the same application identity and settings, and adds Google audio transcription. Handy has no plugin loader. Its original local models and paste implementation remain in place.

**Updates:** the fork's built-in updater is disabled. Official Handy installers will replace the fork and remove the Gemini option. To keep both the cloud engine and future Handy fixes, Sophinvest must bring each official release into this fork and publish a new package. The [maintenance guide](gemini-maintenance.md) lists the changed files and the update procedure.

## Install on Ubuntu

1. Quit Handy from its tray menu.
2. Download the `.deb` from the [Sophinvest Handy releases](https://github.com/sophinvest/Handy/releases).
3. In a terminal in Downloads, install the exact downloaded filename, for example: `sudo apt install ./Handy_0.10.0-gemini.2_amd64.deb`. Use the actual release filename if it differs. Ubuntu installs the package over the existing Handy and resolves its dependencies. Do not delete your existing settings or recordings.
4. Open **Handy** from the app launcher. If you used AppImage instead, download the new AppImage, mark it executable in Properties → Permissions, then run it in place of the old AppImage. The AppImage may need Ubuntu's `libfuse2` or `libfuse2t64` package depending on the Ubuntu version. It also uses the host Vulkan loader (`libvulkan1`), which can be installed with `sudo apt install libvulkan1` if absent. The `.deb` declares this dependency and installs it automatically. Keep the host Vulkan loader rather than bundling a different graphics stack into the AppImage.
5. For rollback, quit Handy and reinstall the official Handy package from its official release page. Your normal local model settings remain available. The Gemini key and saved recovery clips are separate files; rollback does not require deleting them.

The package is unsigned. Ubuntu may identify it as a locally downloaded package; install only the file from the Sophinvest release above.

## Create the free-only key

1. Open [Google AI Studio API keys](https://aistudio.google.com/apikey) and sign in.
2. Choose **Create API key**. Use a new project dedicated to Handy with no linked billing account. Do not select an existing project with billing enabled.
3. Copy the API key and save your own backup in 1Password. Do not send it through Paperclip, chat, email, or a screenshot.
4. Leave billing unlinked. The app never changes Google project billing. A key from a billed project does not become free-only because this app is selected.

## Set up Google dictation

1. Open **Handy → Settings → General → Google Gemini (cloud)**. The same controls are on the Models page. A new install also offers Google during the model-selection step without requiring a local-model download.
2. Paste the key into **Gemini API key** and click **Verify model and save key**. Handy calls Google's `ListModels` endpoint and displays a compatible, non-streaming Transcribe model actually returned for this key. Setup stops if none is available; it does not guess a model ID or silently choose another provider.
3. Check the key-storage line. Handy first uses **GNOME Keyring / Secret Service**. If that service is absent or locked and cannot save, it uses a private file with mode **0600**, inside a mode-700 `gemini-credentials` directory under Handy's application data directory. On a normal Ubuntu install this is `$XDG_DATA_HOME/com.pais.handy/gemini-credentials/api-key`, or `~/.local/share/com.pais.handy/gemini-credentials/api-key` if `XDG_DATA_HOME` is unset. The key is not written to Handy's settings JSON. The storage choice is shown in Settings; the password field clears after saving.
4. Click **Choose Google Gemini**. Keep **Allow local fallback** off. Local speech and language models unload. Cloud capture bypasses voice detection, local live decoding, and the spectrum FFT.
5. Under **Settings → Advanced → Custom Words**, add lender, street, and person names you commonly dictate. The existing list is sent as vocabulary hints with each clip.
6. Click into a text box outside Handy, use the normal dictation shortcut, speak, and release the key. The complete recording is sent once after release as 16 kHz mono PCM WAV. Google returns the verbatim transcript; Handy uses its existing paste-at-cursor path.

## X11 and Wayland

To identify your session, run `echo $XDG_SESSION_TYPE` in a terminal. It prints `x11` or `wayland`. This fork does not change Handy's clipboard, keyboard, shortcut, or paste implementations. Keep the same X11/Wayland paste tools and permissions your current Handy uses. If paste already works in the official app, keep those settings. Test pasting into an ordinary text editor before a business application.

## If Google fails

Quota errors show **Gemini free limit reached**. Offline and timeout errors are reported too. The app makes one request with a 10-second network timeout, then keeps the clip in Handy's `gemini-pending` folder for recovery. There are no automatic network retries. Recordings longer than five minutes are preserved without uploading.

- Click the destination text box and press **Ctrl+Shift+F8** to retry the oldest saved clip once and paste it. Change the shortcut in Google settings if it conflicts with another app.
- To review before pasting, use **Retry oldest clip for preview**, then **Copy transcript**. A preview leaves the WAV saved.
- **Open saved clips** opens the recovery folder. Pending WAVs survive restart and ordinary history cleanup. A successful shortcut retry-and-paste removes exactly that pending file. Delete previewed clips yourself when you no longer need them.
- A local fallback model is available as an explicit opt-in. Choose a downloaded local model, then enable the checkbox. It loads only after a cloud failure and unloads after the clip. It uses CPU and is intentionally off by default.

To return fully to offline dictation, choose a downloaded local model from Handy's normal picker.

## What is tested and what still needs the laptop

Local tests use a mock HTTP server and fixture credentials, never a Google key. They cover model discovery, PCM/WAV encoding, vocabulary, verbatim mode, successful structured/plain responses, quota/auth/server errors, offline failure, timeout, and empty/truncated output. The credential-file test verifies 0600/0700 permissions, replacement, and rejection of unsafe permissions and symbolic links. The 60-second structural buffer comparison checks every sample, including silence. The real recording-sink test also covers a final partial frame and bypasses VAD/FFT.

Mocks do not measure Google's accuracy or network latency. The 2–3 second release-to-paste target is still unmeasured against Google. Native Linux compilation and package smoke results are recorded with each Sophinvest release. Physical microphone capture under full CPU load, Secret Service behavior on Jean's desktop, and X11/Wayland paste still need laptop results. Do not treat a mock transcript as a successful speech-recognition test.

For a deliberate Linux CPU stress test, run `python3 scripts/gemini-cpu-stress.py --seconds 60` while dictating a known 60-second passage. It uses every CPU available to that test process and stops its own workers at the end or on Ctrl+C. Compare every pasted word with the reference and record release-to-paste time. The harness accesses no key, microphone, or clipboard itself. A dedicated capture thread avoids inference-related stalls, but no app can recover samples the operating system or audio driver never delivered.

[Google transcription configuration](https://ai.google.dev/gemini-api/docs/generate-content/transcribe) · [Google ListModels](https://ai.google.dev/api/models)
