import { Languages } from "lucide-react";
import { useI18n } from "../lib/i18n";

export function LanguageToggle() {
  const { lang, setLang, t } = useI18n();
  return (
    <button
      onClick={() => setLang(lang === "en" ? "bn" : "en")}
      title={t("common.language", "Language")}
      aria-label={t("common.language", "Language")}
      className="flex items-center gap-1.5 rounded-lg border border-line bg-surface px-2.5 py-1.5 text-[12px] font-semibold text-navy transition-colors hover:bg-mint/60"
    >
      <Languages size={14} />
      {lang === "en" ? "বাংলা" : "English"}
    </button>
  );
}
