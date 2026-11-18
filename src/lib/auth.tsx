import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import { api, onSessionLost, toApiError, type Principal, type RestoreOutcome } from "./api";
import { useI18n } from "./i18n";

type Phase = "loading" | "setup" | "login" | "ready";

interface Auth {
  phase: Phase;
  principal: Principal | null;
  notice: RestoreOutcome | null;
  fatal: string | null;
  can: (perm: string) => boolean;
  signedIn: (p: Principal) => void;
  signOut: () => Promise<void>;
  reload: () => Promise<void>;
  dismissNotice: () => void;
}

const Ctx = createContext<Auth | null>(null);

export function AuthProvider({ children }: { children: ReactNode }) {
  const [phase, setPhase] = useState<Phase>("loading");
  const [principal, setPrincipal] = useState<Principal | null>(null);
  const [notice, setNotice] = useState<RestoreOutcome | null>(null);
  const [fatal, setFatal] = useState<string | null>(null);
  const { setLang } = useI18n();

  const reload = useCallback(async () => {
    try {
      const st = await api.setupStatus();
      if (st.startup_notice) setNotice(st.startup_notice);
      if (st.needs_setup) return setPhase("setup");
      const p = await api.session();
      setPrincipal(p);
      setPhase(p ? "ready" : "login");
    } catch (e) {
      setFatal(toApiError(e).message);
    }
  }, []);

  useEffect(() => {
    const t = setTimeout(() => void reload(), 0);
    return () => clearTimeout(t);
  }, [reload]);

  useEffect(
    () =>
      onSessionLost(() => {
        setPrincipal(null);
        setPhase("login");
      }),
    [],
  );

  const signedIn = useCallback(
    (p: Principal) => {
      setPrincipal(p);
      setPhase("ready");
      api
        .branding()
        .then((b) => (b.language === "bn" || b.language === "en" ? setLang(b.language) : undefined))
        .catch(() => undefined);
    },
    [setLang],
  );

  const signOut = useCallback(async () => {
    try {
      await api.logout();
    } finally {
      setPrincipal(null);
      setPhase("login");
    }
  }, []);

  const value: Auth = {
    phase,
    principal,
    notice,
    fatal,
    can: (perm) => principal?.permissions.includes(perm) ?? false,
    signedIn,
    signOut,
    reload,
    dismissNotice: () => setNotice(null),
  };
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useAuth(): Auth {
  const v = useContext(Ctx);
  if (!v) throw new Error("useAuth outside AuthProvider");
  return v;
}
