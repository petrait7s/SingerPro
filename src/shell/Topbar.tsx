import { Search } from "lucide-react";
import { NAV, type PageId } from "./nav";

export function Topbar({ page }: { page: PageId }) {
  const item = NAV.find((n) => n.id === page)!;
  return (
    <header className="flex h-14 shrink-0 items-center justify-between border-b border-line bg-surface px-6">
      <div>
        <p className="text-[11px] uppercase tracking-[0.1em] text-ink-2">{item.group}</p>
        <h1 className="text-[18px] font-bold leading-tight text-navy">{item.label}</h1>
      </div>
      <label className="flex w-[320px] items-center gap-2 rounded-lg border border-line bg-pearl px-3 py-1.5 text-ink-2 focus-within:border-teal">
        <Search size={15} />
        <input
          disabled
          aria-label="Global search (available from P2)"
          placeholder="Global search — available once data modules ship"
          className="w-full bg-transparent text-[12px] outline-none placeholder:text-slate-400"
        />
      </label>
    </header>
  );
}
