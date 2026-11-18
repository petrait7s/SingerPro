import { useState } from "react";
import { Dashboard } from "../pages/Dashboard";
import { ModulePage } from "../pages/ModulePage";
import { AboutDialog } from "./AboutDialog";
import type { PageId } from "./nav";
import { Sidebar } from "./Sidebar";
import { Topbar } from "./Topbar";

export function App() {
  const [page, setPage] = useState<PageId>("dashboard");
  const [collapsed, setCollapsed] = useState(false);
  const [about, setAbout] = useState(false);
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
        <Topbar page={page} />
        <main className="flex-1 overflow-y-auto p-6">
          {page === "dashboard" ? <Dashboard /> : <ModulePage id={page} />}
        </main>
      </div>
      {about && <AboutDialog onClose={() => setAbout(false)} />}
    </div>
  );
}
