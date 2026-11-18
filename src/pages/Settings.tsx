import { useState } from "react";
import { useAuth } from "../lib/auth";
import { useI18n } from "../lib/i18n";
import { AuditTab } from "./settings/AuditTab";
import { BackupTab } from "./settings/BackupTab";
import { FinanceTab } from "./settings/FinanceTab";
import { InstallmentTab } from "./settings/InstallmentTab";
import { ShopTab } from "./settings/ShopTab";

type Tab = "shop" | "finance" | "installments" | "backup" | "audit";

export function SettingsPage() {
  const { t } = useI18n();
  const { can } = useAuth();
  const tabs: { id: Tab; label: string; ok: boolean }[] = [
    { id: "shop", label: t("settings.shop", "Shop identity"), ok: can("settings.view") },
    { id: "finance", label: t("settings.finance", "Currency & tax"), ok: can("settings.view") },
    {
      id: "installments",
      label: t("settings.installments", "Installments"),
      ok: can("settings.view"),
    },
    {
      id: "backup",
      label: t("settings.backup", "Backup & restore"),
      ok: can("backup.create") || can("backup.restore"),
    },
    { id: "audit", label: t("settings.audit", "Audit log"), ok: can("audit.view") },
  ];
  const visible = tabs.filter((x) => x.ok);
  const [tab, setTab] = useState<Tab>(visible[0]?.id ?? "shop");
  const current = visible.find((x) => x.id === tab)?.id ?? visible[0]?.id;
  return (
    <div className="space-y-4">
      <div className="flex gap-1 border-b border-line">
        {visible.map((x) => (
          <button
            key={x.id}
            onClick={() => setTab(x.id)}
            aria-current={current === x.id}
            className={`-mb-px border-b-2 px-4 py-2 text-[13px] font-semibold ${current === x.id ? "border-teal text-navy" : "border-transparent text-ink-2 hover:text-navy"}`}
          >
            {x.label}
          </button>
        ))}
      </div>
      {current === "shop" && <ShopTab />}
      {current === "finance" && <FinanceTab />}
      {current === "installments" && <InstallmentTab />}
      {current === "backup" && <BackupTab />}
      {current === "audit" && <AuditTab />}
    </div>
  );
}
