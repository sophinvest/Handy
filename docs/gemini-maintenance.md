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

`.github/workflows/build.yml` also adds the Linux D-Bus development dependency for Secret Service. New files are the isolated Gemini modules/tests, shared cloud settings component, bounded CPU-load harness and these guides.

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
