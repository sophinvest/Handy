import React, { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { useModelStore } from "@/stores/modelStore";
import { ShortcutInput } from "./ShortcutInput";
import { SavedClips } from "./SavedClips";
import { SettingsGroup } from "../ui/SettingsGroup";

export const GEMINI_ENGINE = "google-gemini-cloud";
type Status = {
  configured: boolean;
  credential_storage: "keyring" | "file";
  model: string | null;
  live_model: string | null;
  mode: string;
  chunk_seconds: number;
  rpm: number | null;
  rpd: number | null;
  requests_today: number;
  fallback: boolean;
  fallback_model: string;
  pending: number;
};

export const GeminiSettings: React.FC<{
  onActivated?: () => void;
  disabled?: boolean;
}> = ({ onActivated, disabled = false }) => {
  const { t } = useTranslation();
  const { currentModel, selectModel } = useModelStore();
  const [status, setStatus] = useState<Status>();
  const [key, setKey] = useState("");
  const [working, setWorking] = useState(false);
  const [message, setMessage] = useState("");
  const busy = useRef(false);
  const [mode, setMode] = useState("auto");
  const [chunkSeconds, setChunkSeconds] = useState(60);
  const [rpm, setRpm] = useState("");
  const [rpd, setRpd] = useState("");
  const refresh = async (syncForm = true) => {
    const next = await invoke<Status>("gemini_status");
    setStatus(next);
    // Native select popups return focus to the window. Background status
    // refreshes must not replace the user's unsaved selection or quota edits.
    if (syncForm) {
      setMode(next.mode);
      setChunkSeconds(next.chunk_seconds);
      setRpm(next.rpm?.toString() ?? "");
      setRpd(next.rpd?.toString() ?? "");
    }
  };
  useEffect(() => {
    if (disabled) return;
    const unlisten = listen("gemini-clips-changed", () => {
      refresh(false).catch(() => setMessage(t("gemini.settingsError")));
    });
    const refreshOnFocus = () => {
      refresh(false).catch(() => setMessage(t("gemini.settingsError")));
    };
    window.addEventListener("focus", refreshOnFocus);
    return () => {
      unlisten.then((stop) => stop());
      window.removeEventListener("focus", refreshOnFocus);
    };
  }, [disabled]);
  useEffect(() => {
    if (!disabled) refresh().catch(() => setMessage(t("gemini.settingsError")));
  }, [disabled]);
  const run = async (action: () => Promise<void>) => {
    if (busy.current) return;
    busy.current = true;
    setWorking(true);
    setMessage("");
    try {
      await action();
      await refresh();
    } catch (error) {
      setMessage(String(error));
    } finally {
      busy.current = false;
      setWorking(false);
    }
  };
  const locked = working || disabled;
  return (
    <SettingsGroup title={t("gemini.title")}>
      <div className="space-y-3 p-4">
        <p className="text-sm text-ink-2">{t("gemini.description")}</p>
        <p className="text-sm text-ink-2">{t("gemini.privacy")}</p>
        <label className="block text-sm">
          {t("gemini.key")}
          <input
            className="block w-full border border-line rounded p-2 mt-1"
            type="password"
            autoComplete="off"
            spellCheck={false}
            value={key}
            onChange={(e) => setKey(e.target.value)}
            disabled={locked}
          />
        </label>
        <button
          className="border border-line rounded px-3 py-2"
          disabled={locked || !key.trim()}
          onClick={() =>
            run(async () => {
              const value = key;
              setKey("");
              const verified = await invoke<string[]>("gemini_save_key", {
                key: value,
              });
              setMessage(t("gemini.verified", { model: verified[0] }));
            })
          }
        >
          {t("gemini.save")}
        </button>
        {status?.configured && (
          <p className="text-sm">
            {t(`gemini.storage.${status.credential_storage}`)}
          </p>
        )}
        {status?.model && (
          <p className="text-sm">
            {t("gemini.verified", { model: status.model })}
          </p>
        )}
        <button
          className="block border border-line rounded px-3 py-2"
          disabled={locked || !status?.configured || !status.model}
          onClick={() =>
            run(async () => {
              if (!(await selectModel(GEMINI_ENGINE)))
                throw new Error(t("gemini.switchError"));
              if (onActivated) {
                await commands.completeOnboarding();
                onActivated();
              }
            })
          }
        >
          {t(
            currentModel === GEMINI_ENGINE
              ? "gemini.selected"
              : "gemini.choose",
          )}
        </button>
        <p className="text-sm">
          {t("gemini.liveModel", {
            model: status?.live_model ?? t("gemini.liveUnavailable"),
          })}
        </p>
        <label className="block text-sm">
          {t("gemini.mode")}
          <select
            className="block border border-line rounded p-2 w-full"
            value={mode}
            disabled={locked}
            onChange={(e) => setMode(e.target.value)}
          >
            <option value="auto">{t("gemini.modeAuto")}</option>
            <option value="chunks">{t("gemini.modeChunks")}</option>
            <option value="single">{t("gemini.modeSingle")}</option>
          </select>
        </label>
        <label className="block text-sm">
          {t("gemini.chunkTarget")}
          <select
            className="block border border-line rounded p-2 w-full"
            value={chunkSeconds}
            disabled={locked}
            onChange={(e) => setChunkSeconds(Number(e.target.value))}
          >
            {[30, 60, 90].map((seconds) => (
              <option key={seconds} value={seconds}>
                {t("gemini.chunkSeconds", { count: seconds })}
              </option>
            ))}
          </select>
        </label>
        <p className="text-sm text-ink-2">{t("gemini.chunkHint")}</p>
        <p className="text-sm text-ink-2">{t("gemini.quotaHint")}</p>
        <label className="block text-sm">
          {t("gemini.rpm")}
          <input
            className="block border border-line rounded p-2 w-full"
            type="number"
            min="1"
            value={rpm}
            disabled={locked}
            onChange={(e) => setRpm(e.target.value)}
          />
        </label>
        <label className="block text-sm">
          {t("gemini.rpd")}
          <input
            className="block border border-line rounded p-2 w-full"
            type="number"
            min="1"
            value={rpd}
            disabled={locked}
            onChange={(e) => setRpd(e.target.value)}
          />
        </label>
        <button
          className="border border-line rounded px-3 py-2"
          disabled={locked}
          onClick={() =>
            run(async () => {
              await invoke("gemini_configure_stream", {
                mode,
                chunkSeconds,
                rpm: rpm ? Number(rpm) : null,
                rpd: rpd ? Number(rpd) : null,
              });
            })
          }
        >
          {t("gemini.saveMode")}
        </button>
        <p className="text-sm">
          {t("gemini.usage", { count: status?.requests_today ?? 0 })}
        </p>
        {status?.rpd && status.requests_today >= status.rpd * 0.8 ? (
          <p role="alert" className="text-sm">
            {t("gemini.nearQuota")}
          </p>
        ) : null}
        <p className="text-sm text-ink-2">{t("gemini.pasteRelease")}</p>
        <p className="text-sm text-ink-2">{t("gemini.noLocalInference")}</p>
        <ShortcutInput shortcutId="gemini_retry" grouped />
        <SavedClips
          disabled={locked}
          canRetry={currentModel === GEMINI_ENGINE}
        />
        <p role="status" className="text-sm whitespace-pre-wrap">
          {working ? t("gemini.working") : message}
        </p>
      </div>
    </SettingsGroup>
  );
};
