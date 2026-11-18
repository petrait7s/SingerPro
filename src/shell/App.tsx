import { useState } from "react";
import { Dashboard } from "../pages/Dashboard";
import { ModulePage } from "../pages/ModulePage";
import { UsersPage } from "../pages/Users";
import { SettingsPage } from "../pages/Settings";
import { ForcedChangeScreen, LoginScreen, SetupScreen } from "../pages/AuthScreens";
import { AboutDialog } from "./AboutDialog";
import { NAV, type PageId } from "./nav";
import { Sidebar } from "./Sidebar";
import { Topbar } from "./Topbar";
import { AuthProvider, useAuth } from "../lib/auth";
import { I18nProvider } from "../lib/i18n";
import { ToastProvider } from "../ui/controls";

function Shell() {
  const { principal, can } = useAuth();
  const [page, setPage] = useState<PageId>("dashboard");
  const [collapsed, setCollapsed] = useState(false);
  const [about, setAbout] = useState(false);

  // Never render a page the user may not open (the backend enforces it regardless).
  const requested = NAV.find((n) => n.id === page)!;
  const shown: PageId =
    requested.anyOf.length === 0 || requested.anyOf.some(can) ? page : "dashboard";

  return (
    <div className="flex h-full">
      <Sidebar
        page={page}
        onPage={setPage}
        collapsed={collapsed}
        onToggle={() => setCollapsed((c) => !c)}
        onAbout={() => setAbout(true)}
      />
      <div className="flex min-w-0 flex-1 flex-col">
        <Topbar page={shown} />
        <main className="flex-1 overflow-y-auto p-6" key={principal?.user_id}>
          {page === "dashboard" ? (
            <Dashboard />
          ) : page === "staff" ? (
            <UsersPage />
          ) : page === "settings" ? (
            <SettingsPage />
          ) : (
            <ModulePage id={page} />
          )}
        </main>
      </div>
      {about && <AboutDialog onClose={() => setAbout(false)} />}
    </div>
  );
}

function Gate() {
  const { phase, principal, fatal } = useAuth();
  if (fatal) {
    return (
      <div
        role="alert"
        className="m-8 rounded-lg border border-red-200 bg-red-50 p-4 text-[13px] text-danger"
      >
        {fatal}
      </div>
    );
  }
  if (phase === "loading")
    return <div className="grid h-full place-items-center text-ink-2">…</div>;
  if (phase === "setup") return <SetupScreen />;
  if (phase === "login") return <LoginScreen />;
  if (principal?.must_change_password) return <ForcedChangeScreen />;
  return <Shell />;
}

export function App() {
  return (
    <I18nProvider>
      <ToastProvider>
        <AuthProvider>
          <Gate />
        </AuthProvider>
      </ToastProvider>
    </I18nProvider>
  );
}
