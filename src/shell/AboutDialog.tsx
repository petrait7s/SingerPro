import { useEffect, useState } from "react";
import { api, toApiError, type AppInfo } from "../lib/api";
import { PetraMark, SingerProWordmark } from "../ui/BrandMark";

export function AboutDialog({ onClose }: { onClose: () => void }) {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    api
      .appInfo()
      .then(setInfo)
      .catch((e) => setErr(toApiError(e).message));
    const k = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", k);
    return () => window.removeEventListener("keydown", k);
  }, [onClose]);
  return (
    <div
      className="fixed inset-0 z-50 grid place-items-center bg-navy/50 backdrop-blur-[2px]"
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="About Singer Pro"
        onClick={(e) => e.stopPropagation()}
        className="w-[380px] rounded-2xl bg-surface p-6 shadow-lift"
      >
        <SingerProWordmark />
        <p className="mt-1 text-[12px] text-ink-2">Offline dealer ERP</p>
        <div className="mt-5 flex items-center gap-3 rounded-xl bg-mint/60 p-3">
          <PetraMark size={36} />
          <div>
            <p className="text-[11px] text-ink-2">Developed by</p>
            <p className="text-[13px] font-semibold text-navy">PETRA Software Solutions</p>
          </div>
        </div>
        <dl className="mt-4 space-y-1 text-[12px] text-ink-2">
          <div className="flex justify-between">
            <dt>Version</dt>
            <dd>{info?.version ?? "—"}</dd>
          </div>
          <div className="flex justify-between gap-4">
            <dt>Data folder</dt>
            <dd className="truncate" title={info?.data_dir}>
              {info?.data_dir ?? "—"}
            </dd>
          </div>
        </dl>
        {err && <p className="mt-3 text-[12px] text-danger">{err}</p>}
        <button
          onClick={onClose}
          className="mt-5 w-full rounded-lg bg-navy py-2 text-[13px] font-semibold text-white transition-colors hover:bg-navy-2"
        >
          Close
        </button>
      </div>
    </div>
  );
}
