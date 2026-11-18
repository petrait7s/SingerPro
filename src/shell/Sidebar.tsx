import { ChevronsLeft, ChevronsRight, Info } from "lucide-react";
import { NAV, type PageId } from "./nav";
import { PetraMark, SingerProWordmark } from "../ui/BrandMark";

export function Sidebar({
  page,
  onPage,
  collapsed,
  onToggle,
  onAbout,
}: {
  page: PageId;
  onPage: (p: PageId) => void;
  collapsed: boolean;
  onToggle: () => void;
  onAbout: () => void;
}) {
  const groups = [...new Set(NAV.map((n) => n.group))];
  return (
    <aside
      className={`flex h-full shrink-0 flex-col bg-navy text-slate-300 transition-[width] duration-200 ${collapsed ? "w-[68px]" : "w-[236px]"}`}
    >
      <div className="flex h-14 items-center gap-2 border-b border-white/10 px-4">
        <div className="grid size-8 shrink-0 place-items-center rounded-lg bg-(image:--gradient-liquid) font-bold text-white">
          S
        </div>
        {!collapsed && <SingerProWordmark light />}
      </div>
      <nav className="flex-1 overflow-y-auto px-2 py-3" aria-label="Main">
        {groups.map((g) => (
          <div key={g} className="mb-3">
            {!collapsed && (
              <p className="px-3 pb-1 text-[10px] font-semibold uppercase tracking-[0.12em] text-slate-500">
                {g}
              </p>
            )}
            {NAV.filter((n) => n.group === g).map((n) => {
              const active = n.id === page;
              const Icon = n.icon;
              return (
                <button
                  key={n.id}
                  onClick={() => onPage(n.id)}
                  title={collapsed ? n.label : undefined}
                  aria-current={active ? "page" : undefined}
                  className={`relative mb-0.5 flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-[13px] font-medium transition-colors duration-150 ${
                    active ? "bg-navy-2 text-white" : "hover:bg-navy-1 hover:text-white"
                  }`}
                >
                  {active && (
                    <span className="absolute inset-y-1.5 left-0 w-[3px] rounded-full bg-teal" />
                  )}
                  <Icon size={18} className={active ? "text-teal-glow" : ""} />
                  {!collapsed && <span className="truncate">{n.label}</span>}
                </button>
              );
            })}
          </div>
        ))}
      </nav>
      <div className="flex items-center justify-between border-t border-white/10 p-2">
        <button
          onClick={onAbout}
          className="flex items-center gap-2 rounded-lg px-2 py-2 text-[12px] hover:bg-navy-1"
          title="About"
        >
          <span className="grid size-6 place-items-center rounded-md bg-pearl">
            <PetraMark size={18} />
          </span>
          {!collapsed && (
            <>
              <span>About</span>
              <Info size={13} />
            </>
          )}
        </button>
        <button
          onClick={onToggle}
          aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
          className="rounded-lg p-2 hover:bg-navy-1"
        >
          {collapsed ? <ChevronsRight size={16} /> : <ChevronsLeft size={16} />}
        </button>
      </div>
    </aside>
  );
}
