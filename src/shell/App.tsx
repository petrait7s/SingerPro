import { useState } from "react";
import { Dashboard } from "../pages/Dashboard";
import { ModulePage } from "../pages/ModulePage";
import { UsersPage } from "../pages/Users";
import { SettingsPage } from "../pages/Settings";
import { ProductsPage } from "../pages/Products";
import { InventoryPage } from "../pages/Inventory";
import { SuppliersPage } from "../pages/Suppliers";
import { PurchasesPage } from "../pages/Purchases";
import { PosPage } from "../pages/Pos";
import { SalesPage } from "../pages/Sales";
import { CustomersPage } from "../pages/Customers";
import { AccountsPage } from "../pages/Accounts";
import { InstallmentsPage } from "../pages/Installments";
import { ReturnsPage } from "../pages/Returns";
import { ExpensesPage } from "../pages/Expenses";
import { ForcedChangeScreen, LoginScreen, SetupScreen } from "../pages/AuthScreens";
import { AboutDialog } from "./AboutDialog";
import { NAV, type PageId } from "./nav";
import { Sidebar } from "./Sidebar";
import { Topbar } from "./Topbar";
import { AuthProvider, useAuth } from "../lib/auth";
import { I18nProvider } from "../lib/i18n";
import { ToastProvider } from "../ui/controls";

function renderPage(id: PageId) {
  switch (id) {
    case "dashboard":
      return <Dashboard />;
    case "staff":
      return <UsersPage />;
    case "settings":
      return <SettingsPage />;
    case "products":
      return <ProductsPage />;
    case "inventory":
      return <InventoryPage />;
    case "suppliers":
      return <SuppliersPage />;
    case "purchases":
      return <PurchasesPage />;
    case "pos":
      return <PosPage />;
    case "sales":
      return <SalesPage />;
    case "wholesale":
      return <SalesPage fixedMode="wholesale" />;
    case "customers":
      return <CustomersPage />;
    case "accounts":
      return <AccountsPage />;
    case "installments":
      return <InstallmentsPage />;
    case "returns":
      return <ReturnsPage />;
    case "expenses":
      return <ExpensesPage />;
    default:
      return <ModulePage id={id} />;
  }
}

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
          {renderPage(shown)}
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
