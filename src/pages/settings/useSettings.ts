import { useCallback, useEffect, useState } from "react";
import { api, toApiError } from "../../lib/api";
import { useToast } from "../../ui/controls";
import { useI18n } from "../../lib/i18n";

/** Loads settings, tracks a draft, saves only what changed (backend validates + audits). */
export function useSettings() {
  const toast = useToast();
  const { t } = useI18n();
  const [saved, setSaved] = useState<Record<string, string> | null>(null);
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    api
      .settingsGet()
      .then((s) => {
        setSaved(s);
        setDraft(s);
      })
      .catch((e) => setError(toApiError(e).message));
  }, []);
  useEffect(load, [load]);

  const set = (k: string, v: string) => setDraft((d) => ({ ...d, [k]: v }));
  const get = (k: string) => draft[k] ?? "";
  const dirty = saved ? Object.keys(draft).some((k) => draft[k] !== saved[k]) : false;

  async function save(keys: string[], extra?: () => string | null) {
    if (!saved) return;
    const problem = extra?.();
    if (problem) return setError(problem);
    const changes: Record<string, string> = {};
    for (const k of keys) if (draft[k] !== saved[k]) changes[k] = draft[k] ?? "";
    if (Object.keys(changes).length === 0) return toast("ok", t("common.noChanges", "No changes"));
    setBusy(true);
    setError(null);
    try {
      const changed = await api.settingsUpdate(changes);
      toast("ok", `${t("common.saved", "Saved")}: ${changed.length}`);
      load();
    } catch (e) {
      setError(toApiError(e).message);
    } finally {
      setBusy(false);
    }
  }
  return { loaded: saved !== null, get, set, save, busy, error, dirty };
}
