import React, { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { useModelStore } from "@/stores/modelStore";
import { ShortcutInput } from "./ShortcutInput";
import { SettingsGroup } from "../ui/SettingsGroup";

export const GEMINI_ENGINE = "google-gemini-cloud";
type Status = {
  configured: boolean;
  model: string | null;
  fallback: boolean;
  fallback_model: string;
  pending: number;
};

export const GeminiSettings: React.FC<{
  onActivated?: () => void;
  disabled?: boolean;
}> = ({ onActivated, disabled = false }) => {
  const { t } = useTranslation();
  const { models, currentModel, selectModel } = useModelStore();
  const [status, setStatus] = useState<Status>();
  const [key, setKey] = useState("");
  const [working, setWorking] = useState(false);
  const [message, setMessage] = useState("");
  const [preview, setPreview] = useState("");
  const [localModel, setLocalModel] = useState("");
  const refresh = async () => {
    const next = await invoke<Status>("gemini_status");
    setStatus(next);
    setLocalModel(next.fallback_model);
  };
  useEffect(() => {
    if (disabled) return;
    const unlisten = listen("gemini-clips-changed", () => {
      refresh().catch(() => setMessage(t("gemini.settingsError")));
    });
    const refreshOnFocus = () => {
      refresh().catch(() => setMessage(t("gemini.settingsError")));
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
    setWorking(true);
    setMessage("");
    try {
      await action();
      await refresh();
    } catch (error) {
      setMessage(String(error));
    } finally {
      setWorking(false);
    }
  };
  const locals = models.filter(
    (m) => m.id !== GEMINI_ENGINE && m.is_downloaded,
  );
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
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={status?.fallback ?? false}
            disabled={locked || !localModel}
            onChange={(e) =>
              run(async () => {
                await invoke("gemini_set_fallback", {
                  enabled: e.target.checked,
                  model: localModel,
                });
              })
            }
          />
          {t("gemini.fallback")}
        </label>
        <p className="text-sm text-ink-2">{t("gemini.fallbackHint")}</p>
        <select
          aria-label={t("gemini.localModel")}
          className="border border-line rounded p-2 w-full"
          value={localModel}
          disabled={locked}
          onChange={(e) => {
            const model = e.target.value;
            setLocalModel(model);
            run(async () => {
              await invoke("gemini_set_fallback", {
                enabled: status?.fallback ?? false,
                model,
              });
            });
          }}
        >
          <option value="">{t("gemini.localModel")}</option>
          {locals.map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}
            </option>
          ))}
        </select>
        <ShortcutInput shortcutId="gemini_retry" grouped />
        <p className="text-sm">
          {t("gemini.pending", { count: status?.pending ?? 0 })}
        </p>
        <button
          className="border border-line rounded px-3 py-2"
          disabled={
            locked || !status?.pending || currentModel !== GEMINI_ENGINE
          }
          onClick={() =>
            run(async () => {
              setPreview(await invoke<string>("gemini_retry_clip"));
            })
          }
        >
          {t("gemini.retry")}
        </button>
        <button
          className="ms-2 border border-line rounded px-3 py-2"
          disabled={locked}
          onClick={() =>
            run(async () => {
              await invoke("gemini_open_clips");
            })
          }
        >
          {t("gemini.openClips")}
        </button>
        {preview && (
          <>
            <textarea
              className="block w-full border border-line rounded p-2"
              readOnly
              value={preview}
              aria-label={t("gemini.transcript")}
            />
            <button
              className="border border-line rounded px-3 py-2"
              onClick={() =>
                run(async () => {
                  await writeText(preview);
                  setMessage(t("gemini.copied"));
                })
              }
            >
              {t("gemini.copy")}
            </button>
          </>
        )}
        <p role="status" className="text-sm whitespace-pre-wrap">
          {working ? t("gemini.working") : message}
        </p>
      </div>
    </SettingsGroup>
  );
};
