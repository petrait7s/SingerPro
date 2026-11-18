import type { ReactNode } from "react";

export function Card({
  title,
  action,
  children,
  className = "",
}: {
  title?: string;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section
      className={`rounded-(--radius-card) border border-line bg-surface shadow-card transition-shadow duration-200 hover:shadow-lift ${className}`}
    >
      {title && (
        <header className="flex items-center justify-between border-b border-line px-4 py-3">
          <h2 className="text-[13px] font-semibold tracking-wide text-navy">{title}</h2>
          {action}
        </header>
      )}
      <div className="p-4">{children}</div>
    </section>
  );
}

export function Badge({ tone, children }: { tone: "ok" | "bad" | "neutral"; children: ReactNode }) {
  const t = {
    ok: "bg-mint text-teal-deep",
    bad: "bg-red-50 text-danger",
    neutral: "bg-slate-100 text-ink-2",
  }[tone];
  return (
    <span className={`rounded-md px-2 py-0.5 text-[11px] font-semibold ${t}`}>{children}</span>
  );
}

export function EmptyState({ title, hint }: { title: string; hint: string }) {
  return (
    <div className="rounded-lg bg-(image:--gradient-aurora) px-6 py-10 text-center">
      <p className="text-[15px] font-semibold text-navy">{title}</p>
      <p className="mx-auto mt-1 max-w-md text-[13px] text-ink-2">{hint}</p>
    </div>
  );
}
