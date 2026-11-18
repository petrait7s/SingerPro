import { useState } from "react";
import { KeyRound, LogOut, Search, UserCircle } from "lucide-react";
import { NAV, type PageId } from "./nav";
import { useAuth } from "../lib/auth";
import { useI18n } from "../lib/i18n";
import { LanguageToggle } from "./LanguageToggle";
import { Modal } from "../ui/controls";
import { ChangePasswordForm } from "../pages/AuthScreens";

export function Topbar({ page }: { page: PageId }) {
  const item = NAV.find((n) => n.id === page)!;
  const { t } = useI18n();
  const { principal, signOut } = useAuth();
  const [menu, setMenu] = useState(false);
  const [pw, setPw] = useState(false);
  return (
    <header className="flex h-14 shrink-0 items-center justify-between border-b border-line bg-surface px-6">
      <div>
        <p className="text-[11px] uppercase tracking-[0.1em] text-ink-2">
          {t(`nav.${item.group}`, item.group)}
        </p>
        <h1 className="text-[18px] font-bold leading-tight text-navy">
          {t(`nav.${item.id}`, item.label)}
        </h1>
      </div>
      <div className="flex items-center gap-3">
        <label className="flex w-[260px] items-center gap-2 rounded-lg border border-line bg-pearl px-3 py-1.5 text-ink-2">
          <Search size={15} />
          <input
            disabled
            aria-label="Global search (available from P2)"
            placeholder="Search — available once data modules ship"
            className="w-full bg-transparent text-[12px] outline-none placeholder:text-slate-400"
          />
        </label>
        <LanguageToggle />
        <div className="relative">
          <button
            onClick={() => setMenu((m) => !m)}
            aria-haspopup="menu"
            aria-expanded={menu}
            className="flex items-center gap-2 rounded-lg border border-line px-2.5 py-1.5 text-[12px] font-semibold text-navy hover:bg-mint/60"
          >
            <UserCircle size={16} />
            {principal?.full_name}
          </button>
          {menu && (
            <div
              role="menu"
              className="absolute right-0 top-full z-40 mt-1 w-56 rounded-xl border border-line bg-surface p-1 shadow-lift"
            >
              <p className="px-3 py-2 text-[11px] text-ink-2">
                {principal?.username} · {principal?.roles.join(", ")}
              </p>
              <button
                role="menuitem"
                onClick={() => {
                  setMenu(false);
                  setPw(true);
                }}
                className="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-[13px] hover:bg-mint/60"
              >
                <KeyRound size={14} />
                {t("common.changePassword", "Change password")}
              </button>
              <button
                role="menuitem"
                onClick={() => void signOut()}
                className="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-[13px] hover:bg-mint/60"
              >
                <LogOut size={14} />
                {t("common.signOut", "Sign out")}
              </button>
            </div>
          )}
        </div>
      </div>
      {pw && (
        <Modal title={t("common.changePassword", "Change password")} onClose={() => setPw(false)}>
          <ChangePasswordForm onDone={() => void signOut()} onCancel={() => setPw(false)} />
        </Modal>
      )}
    </header>
  );
}
