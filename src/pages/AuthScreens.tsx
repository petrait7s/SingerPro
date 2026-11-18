import { useState, type FormEvent, type ReactNode } from "react";
import { api, toApiError } from "../lib/api";
import { useAuth } from "../lib/auth";
import { useI18n } from "../lib/i18n";
import { Button, Field, Input } from "../ui/controls";
import { PetraMark, SingerProWordmark } from "../ui/BrandMark";
import { LanguageToggle } from "../shell/LanguageToggle";

function Frame({ title, intro, children }: { title: string; intro?: string; children: ReactNode }) {
  const { t } = useI18n();
  return (
    <div className="grid h-full place-items-center bg-(image:--gradient-aurora) p-6">
      <div className="absolute right-6 top-4">
        <LanguageToggle />
      </div>
      <div className="w-[420px] rounded-2xl bg-surface p-8 shadow-lift">
        <SingerProWordmark />
        <p className="mt-0.5 text-[12px] text-ink-2">{t("app.tagline", "Offline dealer ERP")}</p>
        <h1 className="bn-editorial mt-6 text-[20px] font-bold text-navy">{title}</h1>
        {intro && <p className="mt-1 text-[13px] text-ink-2">{intro}</p>}
        <div className="mt-5">{children}</div>
        <div className="mt-6 flex items-center gap-2 border-t border-line pt-4 text-[11px] text-ink-2">
          <span className="grid size-6 place-items-center rounded-md bg-pearl">
            <PetraMark size={18} />
          </span>
          Developed by PETRA Software Solutions
        </div>
      </div>
    </div>
  );
}

export function SetupScreen() {
  const { t } = useI18n();
  const { signedIn } = useAuth();
  const [username, setUsername] = useState("");
  const [fullName, setFullName] = useState("");
  const [pw, setPw] = useState("");
  const [pw2, setPw2] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (pw !== pw2) return setErr(t("setup.mismatch", "Passwords do not match"));
    setBusy(true);
    setErr(null);
    try {
      signedIn(await api.setupOwner(username, fullName, pw));
    } catch (x) {
      setErr(toApiError(x).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Frame
      title={t("setup.title", "Welcome to Singer Pro")}
      intro={t(
        "setup.intro",
        "Create the owner account. There is no default password — you choose it now.",
      )}
    >
      <form onSubmit={submit} className="space-y-3">
        <Field label={t("setup.fullName", "Full name")}>
          {(id) => (
            <Input
              id={id}
              value={fullName}
              onChange={(e) => setFullName(e.target.value)}
              autoFocus
              required
            />
          )}
        </Field>
        <Field label={t("login.username", "Username")} hint="3–32: a–z, 0–9, . _ -">
          {(id) => (
            <Input
              id={id}
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              autoComplete="username"
              required
            />
          )}
        </Field>
        <Field
          label={t("login.password", "Password")}
          hint={t("pw.rules", "At least 10 characters. Avoid common passwords and your username.")}
        >
          {(id) => (
            <Input
              id={id}
              type="password"
              value={pw}
              onChange={(e) => setPw(e.target.value)}
              autoComplete="new-password"
              required
            />
          )}
        </Field>
        <Field label={t("setup.confirm", "Confirm password")} error={err}>
          {(id) => (
            <Input
              id={id}
              type="password"
              value={pw2}
              onChange={(e) => setPw2(e.target.value)}
              autoComplete="new-password"
              required
            />
          )}
        </Field>
        <Button type="submit" variant="primary" busy={busy} className="w-full">
          {t("setup.create", "Create owner account")}
        </Button>
      </form>
    </Frame>
  );
}

export function LoginScreen() {
  const { t } = useI18n();
  const { signedIn, notice, dismissNotice } = useAuth();
  const [username, setUsername] = useState("");
  const [pw, setPw] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setErr(null);
    try {
      signedIn(await api.login(username, pw));
    } catch (x) {
      setErr(toApiError(x).message);
      setPw("");
    } finally {
      setBusy(false);
    }
  }
  return (
    <Frame title={t("login.title", "Sign in")}>
      {notice && (
        <div
          role="status"
          className={`mb-4 rounded-lg px-3 py-2 text-[12px] ${notice.applied ? "bg-mint text-teal-deep" : "bg-red-50 text-danger"}`}
        >
          {notice.message}
          <button onClick={dismissNotice} className="ml-2 underline">
            OK
          </button>
        </div>
      )}
      <form onSubmit={submit} className="space-y-3">
        <Field label={t("login.username", "Username")}>
          {(id) => (
            <Input
              id={id}
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              autoComplete="username"
              autoFocus
              required
            />
          )}
        </Field>
        <Field label={t("login.password", "Password")} error={err}>
          {(id) => (
            <Input
              id={id}
              type="password"
              value={pw}
              onChange={(e) => setPw(e.target.value)}
              autoComplete="current-password"
              required
            />
          )}
        </Field>
        <Button type="submit" variant="primary" busy={busy} className="w-full">
          {t("login.submit", "Sign in")}
        </Button>
      </form>
    </Frame>
  );
}

/** Used both for forced first-login change and voluntary change. Signs the user out on success. */
export function ChangePasswordForm({
  forced,
  onDone,
  onCancel,
}: {
  forced?: boolean;
  onDone: () => void;
  onCancel?: () => void;
}) {
  const { t } = useI18n();
  const [old, setOld] = useState("");
  const [pw, setPw] = useState("");
  const [pw2, setPw2] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  async function submit(e: FormEvent) {
    e.preventDefault();
    if (pw !== pw2) return setErr(t("setup.mismatch", "Passwords do not match"));
    setBusy(true);
    setErr(null);
    try {
      await api.changePassword(old, pw);
      onDone();
    } catch (x) {
      setErr(toApiError(x).message);
    } finally {
      setBusy(false);
    }
  }
  return (
    <form onSubmit={submit} className="space-y-3">
      {forced && (
        <p className="text-[13px] text-ink-2">
          {t("pw.change.intro", "You must choose your own password before continuing.")}
        </p>
      )}
      <Field label={t("pw.current", "Current password")}>
        {(id) => (
          <Input
            id={id}
            type="password"
            value={old}
            onChange={(e) => setOld(e.target.value)}
            autoComplete="current-password"
            required
          />
        )}
      </Field>
      <Field
        label={t("pw.new", "New password")}
        hint={t("pw.rules", "At least 10 characters. Avoid common passwords and your username.")}
      >
        {(id) => (
          <Input
            id={id}
            type="password"
            value={pw}
            onChange={(e) => setPw(e.target.value)}
            autoComplete="new-password"
            required
          />
        )}
      </Field>
      <Field label={t("setup.confirm", "Confirm password")} error={err}>
        {(id) => (
          <Input
            id={id}
            type="password"
            value={pw2}
            onChange={(e) => setPw2(e.target.value)}
            autoComplete="new-password"
            required
          />
        )}
      </Field>
      <div className="flex justify-end gap-2">
        {onCancel && (
          <Button type="button" onClick={onCancel}>
            {t("common.cancel", "Cancel")}
          </Button>
        )}
        <Button type="submit" variant="primary" busy={busy}>
          {t("common.save", "Save")}
        </Button>
      </div>
    </form>
  );
}

export function ForcedChangeScreen() {
  const { t } = useI18n();
  const { signOut } = useAuth();
  return (
    <Frame title={t("pw.change.title", "Choose a new password")}>
      <ChangePasswordForm forced onDone={() => void signOut()} />
    </Frame>
  );
}
