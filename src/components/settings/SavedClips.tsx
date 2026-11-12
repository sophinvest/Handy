import React, { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import type { SavedClip } from "@/bindings";

export function clipDuration(seconds: number): string {
  const total = Math.floor(seconds);
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

export const SavedClips: React.FC<{ disabled: boolean; canRetry: boolean }> = ({
  disabled,
  canRetry,
}) => {
  const { t, i18n } = useTranslation();
  const [clips, setClips] = useState<SavedClip[]>([]);
  const [working, setWorking] = useState(false);
  const [message, setMessage] = useState("");
  const [ready, setReady] = useState(false);
  const busy = useRef(false);
  const generation = useRef(0);
  const refresh = useCallback(async () => {
    const request = ++generation.current;
    const next = await invoke<SavedClip[]>("gemini_list_clips");
    // A slow pre-delete response must not resurrect a stale row.
    if (request === generation.current) {
      setClips(next);
      setReady(true);
    }
  }, []);
  useEffect(() => {
    let mounted = true;
    const update = () =>
      refresh().catch(() => {
        if (mounted) {
          setReady(false);
          setMessage(t("gemini.listError"));
        }
      });
    update();
    const stop = listen("gemini-clips-changed", update);
    window.addEventListener("focus", update);
    return () => {
      mounted = false;
      ++generation.current;
      stop.then((off) => off());
      window.removeEventListener("focus", update);
    };
  }, [refresh, t]);
  const run = async (action: () => Promise<void>, success?: string) => {
    if (busy.current || disabled) return;
    busy.current = true;
    setWorking(true);
    setMessage("");
    try {
      await action();
      if (success) setMessage(success);
    } catch (e) {
      setMessage(String(e));
    } finally {
      try {
        await refresh();
      } catch {
        setReady(false);
        setMessage(t("gemini.listError"));
      }
      busy.current = false;
      setWorking(false);
    }
  };
  const retry = (clip: SavedClip | null) =>
    run(async () => {
      await invoke("gemini_retry_clip", { clipId: clip?.id ?? null });
    }, t("gemini.delivered"));
  const metadata = (clip: SavedClip) =>
    t("gemini.clipTime", {
      time: new Date(clip.saved_at_ms).toLocaleString(i18n.language, {
        dateStyle: "short",
        timeStyle: "short",
      }),
      duration:
        clip.duration_seconds === null
          ? t("gemini.unknownDuration")
          : clipDuration(clip.duration_seconds),
    });
  const locked = disabled || working || !ready;
  const latest = clips[0];
  return (
    <section className="space-y-3" aria-label={t("gemini.clipsTitle")}>
      <p className="text-sm">{t("gemini.pending", { count: clips.length })}</p>
      <p className="text-sm text-ink-2">{t("gemini.retryHint")}</p>
      <div className="flex flex-wrap items-center gap-2">
        <button
          className="border border-line rounded px-3 py-2"
          disabled={locked || !latest || !canRetry}
          onClick={() => latest && retry(null)}
        >
          {t("gemini.retry")}
        </button>
        {latest && (
          <span className="text-sm text-ink-2">{metadata(latest)}</span>
        )}
      </div>
      {ready && !latest && <p className="text-sm">{t("gemini.emptyClips")}</p>}
      {clips.length > 0 && (
        <ul
          className="space-y-2 max-h-64 overflow-y-auto"
          aria-label={t("gemini.clipsTitle")}
        >
          {clips.map((clip) => (
            <li
              key={clip.id}
              data-clip-id={clip.id}
              className="flex flex-wrap items-center gap-2 border border-line rounded p-2"
            >
              <span className="text-sm grow">{metadata(clip)}</span>
              <button
                className="border border-line rounded px-3 py-2"
                disabled={locked || !canRetry}
                onClick={() => retry(clip)}
              >
                {t("gemini.retryClip")}
              </button>
              <button
                className="border border-line rounded px-3 py-2"
                disabled={locked}
                onClick={() =>
                  run(async () => {
                    await invoke("gemini_delete_clip", {
                      clipId: clip?.id ?? null,
                    });
                  }, t("gemini.deleted"))
                }
              >
                {t("gemini.deleteClip")}
              </button>
            </li>
          ))}
        </ul>
      )}
      <button
        className="border border-line rounded px-3 py-2"
        disabled={disabled || working}
        onClick={() =>
          run(async () => {
            await invoke("gemini_open_clips");
          })
        }
      >
        {t("gemini.openClips")}
      </button>
      <p role="status" className="text-sm whitespace-pre-wrap">
        {working ? t("gemini.working") : message}
      </p>
    </section>
  );
};
