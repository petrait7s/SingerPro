import {
  createContext,
  useCallback,
  useEffect,
  useContext,
  useMemo,
  useState,
  type ReactNode,
} from "react";

export type Lang = "en" | "bn";

const bn: Record<string, string> = {
  "app.tagline": "অফলাইন ডিলার ইআরপি",
  "login.title": "সাইন ইন",
  "login.username": "ইউজারনেম",
  "login.password": "পাসওয়ার্ড",
  "login.submit": "সাইন ইন করুন",
  "setup.title": "সিঙ্গার প্রো-তে স্বাগতম",
  "setup.intro": "মালিকের অ্যাকাউন্ট তৈরি করুন। কোনো ডিফল্ট পাসওয়ার্ড নেই — এখনই আপনি নিজে বেছে নিন।",
  "setup.fullName": "পুরো নাম",
  "setup.confirm": "পাসওয়ার্ড নিশ্চিত করুন",
  "setup.create": "মালিকের অ্যাকাউন্ট তৈরি করুন",
  "setup.mismatch": "পাসওয়ার্ড দুটি মেলেনি",
  "pw.rules": "কমপক্ষে ১০টি অক্ষর। সাধারণ পাসওয়ার্ড ও ইউজারনেম এড়িয়ে চলুন।",
  "pw.change.title": "নতুন পাসওয়ার্ড দিন",
  "pw.change.intro": "চালিয়ে যাওয়ার আগে আপনাকে নিজের পাসওয়ার্ড বেছে নিতে হবে।",
  "pw.current": "বর্তমান পাসওয়ার্ড",
  "pw.new": "নতুন পাসওয়ার্ড",
  "common.save": "সংরক্ষণ করুন",
  "common.cancel": "বাতিল",
  "common.close": "বন্ধ করুন",
  "common.create": "তৈরি করুন",
  "common.signOut": "সাইন আউট",
  "common.changePassword": "পাসওয়ার্ড পরিবর্তন",
  "common.language": "ভাষা",
  "common.loading": "লোড হচ্ছে…",
  "common.saved": "সংরক্ষিত হয়েছে",
  "common.noChanges": "কোনো পরিবর্তন নেই",
  "common.active": "সক্রিয়",
  "common.inactive": "নিষ্ক্রিয়",
  "common.reason": "কারণ",
  "common.name": "নাম",
  "common.roles": "ভূমিকা",
  "common.actions": "কাজ",
  "nav.Overview": "সংক্ষিপ্ত বিবরণ",
  "nav.Sales": "বিক্রয়",
  "nav.Relationships": "সম্পর্ক",
  "nav.Stock": "মজুত",
  "nav.Operations": "পরিচালনা",
  "nav.Finance": "অর্থ",
  "nav.Admin": "প্রশাসন",
  "nav.dashboard": "ড্যাশবোর্ড",
  "nav.pos": "নতুন বিক্রয় / পিওএস",
  "nav.sales": "বিক্রয়",
  "nav.wholesale": "পাইকারি",
  "nav.installments": "কিস্তি",
  "nav.customers": "গ্রাহক",
  "nav.suppliers": "সরবরাহকারী",
  "nav.products": "পণ্য",
  "nav.inventory": "মজুত",
  "nav.purchases": "ক্রয়",
  "nav.returns": "ফেরত ও ওয়ারেন্টি",
  "nav.expenses": "খরচ",
  "nav.accounts": "হিসাব",
  "nav.reports": "রিপোর্ট",
  "nav.staff": "কর্মী ও ব্যবহারকারী",
  "nav.settings": "সেটিংস",
  "settings.shop": "দোকানের পরিচয়",
  "settings.finance": "মুদ্রা ও কর",
  "settings.installments": "কিস্তি",
  "settings.backup": "ব্যাকআপ ও রিস্টোর",
  "settings.audit": "অডিট লগ",
  "shop.name": "দোকানের নাম",
  "shop.address": "ঠিকানা",
  "shop.phone": "ফোন",
  "shop.email": "ইমেইল",
  "shop.footer": "ইনভয়েসের নিচের লেখা",
  "shop.prefix": "ইনভয়েস প্রিফিক্স",
  "shop.logo": "দোকানের লোগো",
  "shop.logo.upload": "লোগো আপলোড",
  "shop.logo.remove": "লোগো মুছুন",
  "fin.code": "মুদ্রার কোড",
  "fin.symbol": "মুদ্রার চিহ্ন",
  "fin.taxEnabled": "কর চালু করুন",
  "fin.taxLabel": "করের নাম",
  "fin.taxRate": "ডিফল্ট করের হার (%)",
  "fin.taxIncl": "মূল্যে কর অন্তর্ভুক্ত",
  "inst.markup": "ডিফল্ট মার্কআপ (%)",
  "inst.recog": "মার্কআপ স্বীকৃতি",
  "inst.grace": "গ্রেস দিন",
  "inst.lateEnabled": "বিলম্ব ফি চালু করুন",
  "inst.lateFee": "বিলম্ব ফি (%)",
  "backup.now": "এখনই ব্যাকআপ নিন",
  "backup.choose": "ফোল্ডার বেছে নিয়ে ব্যাকআপ",
  "backup.last": "সর্বশেষ ব্যাকআপ",
  "backup.never": "কখনও নয়",
  "backup.history": "ইতিহাস",
  "backup.restore": "ব্যাকআপ থেকে রিস্টোর",
  "backup.overdue": "ব্যাকআপ নেওয়ার সময় হয়েছে",
  "users.title": "ব্যবহারকারী",
  "users.new": "নতুন ব্যবহারকারী",
  "users.roles": "ভূমিকা ও অনুমতি",
  "users.username": "ইউজারনেম",
  "users.fullName": "পুরো নাম",
  "users.tempPassword": "অস্থায়ী পাসওয়ার্ড",
  "users.lastLogin": "সর্বশেষ লগইন",
  "dash.db": "ডাটাবেস",
  "dash.integrity": "অখণ্ডতা",
  "dash.kpi": "ব্যবসায়িক সূচক",
  "dash.healthy": "সুস্থ",
};

const dicts: Record<Lang, Record<string, string>> = { en: {}, bn };

interface I18n {
  lang: Lang;
  setLang: (l: Lang) => void;
  /** Returns the translation, or the English fallback. */
  t: (key: string, fallback: string) => string;
}

const Ctx = createContext<I18n>({ lang: "en", setLang: () => {}, t: (_k, f) => f });

function initialLang(): Lang {
  try {
    return localStorage.getItem("sp.lang") === "bn" ? "bn" : "en";
  } catch {
    return "en";
  }
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(initialLang);
  const setLang = useCallback((l: Lang) => {
    setLangState(l);
    try {
      localStorage.setItem("sp.lang", l);
    } catch {
      /* storage unavailable: language just won't persist */
    }
  }, []);
  const value = useMemo<I18n>(
    () => ({ lang, setLang, t: (k, f) => dicts[lang][k] ?? f }),
    [lang, setLang],
  );
  useEffect(() => {
    document.documentElement.lang = lang;
  }, [lang]);
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export const useI18n = () => useContext(Ctx);
