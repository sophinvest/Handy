import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import type { TraceStatus } from "@/bindings";

// Opt-in detailed Google log (SOP-10811). Off by default.
export const GeminiTraceLog: React.FC<{ disabled?: boolean }> = ({
  disabled = false,
}) => {
  const { t } = useTranslation();
  const [status, setStatus] = useState<TraceStatus>();
  const [message, setMessage] = useState("");

  useEffect(() => {
    commands.geminiTraceStatus().then((r) => {
      if (r.status === "ok") setStatus(r.data);
    });
  }, []);

  const toggle = async (enabled: boolean) => {
    const r = await commands.geminiTraceSet(enabled);
    if (r.status === "ok") {
      setStatus(r.data);
      setMessage("");
    } else setMessage(r.error);
  };
  const copy = async () => {
    const r = await commands.geminiTraceCopy();
    if (r.status !== "ok") return setMessage(r.error);
    try {
      await navigator.clipboard.writeText(r.data);
      setMessage(t("gemini.trace.copied"));
    } catch {
      setMessage(t("gemini.trace.copyFailed"));
    }
  };

  return (
    <div className="space-y-2">
      <label className="flex items-start gap-2 text-sm">
        <input
          type="checkbox"
          className="mt-1"
          checked={status?.enabled ?? false}
          disabled={disabled}
          onChange={(e) => toggle(e.target.checked)}
        />
        <span>
          {t("gemini.trace.toggle")}
          <span className="block text-ink-2">{t("gemini.trace.hint")}</span>
        </span>
      </label>
      {status?.enabled && (
        <>
          <p className="text-sm text-ink-2 break-all">
            {t("gemini.trace.file", { file: status.file })}
          </p>
          <div className="flex gap-2">
            <button
              className="border border-line rounded px-3 py-2"
              onClick={copy}
            >
              {t("gemini.trace.copy")}
            </button>
            <button
              className="border border-line rounded px-3 py-2"
              onClick={() => commands.geminiTraceOpen()}
            >
              {t("gemini.trace.open")}
            </button>
          </div>
        </>
      )}
      {message && <p className="text-sm whitespace-pre-wrap">{message}</p>}
    </div>
  );
};
