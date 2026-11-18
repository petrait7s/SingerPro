import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useId,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
  type SelectHTMLAttributes,
} from "react";
import { X } from "lucide-react";

type Variant = "primary" | "secondary" | "danger" | "ghost";
const VARIANT: Record<Variant, string> = {
  primary: "bg-teal text-navy font-semibold hover:bg-teal-glow",
  secondary: "border border-line bg-surface text-navy hover:bg-mint/60",
  danger: "bg-danger text-white hover:bg-red-800",
  ghost: "text-ink-2 hover:bg-slate-100",
};

export function Button({
  variant = "secondary",
  busy,
  className = "",
  children,
  disabled,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant; busy?: boolean }) {
  return (
    <button
      {...rest}
      disabled={disabled || busy}
      className={`inline-flex items-center justify-center gap-2 rounded-lg px-3.5 py-2 text-[13px] transition-colors duration-150 disabled:cursor-not-allowed disabled:opacity-50 ${VARIANT[variant]} ${className}`}
    >
      {busy ? "…" : children}
    </button>
  );
}

export function Field({
  label,
  hint,
  error,
  children,
}: {
  label: string;
  hint?: string;
  error?: string | null;
  children: (id: string) => ReactNode;
}) {
  const id = useId();
  return (
    <div className="space-y-1">
      <label htmlFor={id} className="block text-[12px] font-semibold text-navy">
        {label}
      </label>
      {children(id)}
      {error ? (
        <p role="alert" className="text-[12px] text-danger">
          {error}
        </p>
      ) : hint ? (
        <p className="text-[11px] text-ink-2">{hint}</p>
      ) : null}
    </div>
  );
}

const INPUT =
  "w-full rounded-lg border border-line bg-surface px-3 py-2 text-[13px] text-ink outline-none transition-colors focus:border-teal disabled:bg-slate-50";

export function Input(props: InputHTMLAttributes<HTMLInputElement>) {
  return <input {...props} className={`${INPUT} ${props.className ?? ""}`} />;
}

export function Select(props: SelectHTMLAttributes<HTMLSelectElement>) {
  return <select {...props} className={`${INPUT} ${props.className ?? ""}`} />;
}

export function Toggle({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <label className="flex cursor-pointer items-center gap-2 text-[13px] text-navy">
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        className="size-4 accent-teal"
      />
      {label}
    </label>
  );
}

export function Modal({
  title,
  onClose,
  children,
  width = 460,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
  width?: number;
}) {
  useEffect(() => {
    const k = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", k);
    return () => window.removeEventListener("keydown", k);
  }, [onClose]);
  return (
    <div
      className="fixed inset-0 z-50 grid place-items-center bg-navy/50 backdrop-blur-[2px]"
      onMouseDown={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onMouseDown={(e) => e.stopPropagation()}
        style={{ width }}
        className="max-h-[90vh] overflow-y-auto rounded-2xl bg-surface p-5 shadow-lift"
      >
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-[15px] font-bold text-navy">{title}</h2>
          <button
            onClick={onClose}
            aria-label="Close"
            className="rounded-md p-1 text-ink-2 hover:bg-slate-100"
          >
            <X size={16} />
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}

interface Toast {
  id: number;
  tone: "ok" | "bad";
  text: string;
}
const ToastCtx = createContext<(tone: Toast["tone"], text: string) => void>(() => {});
export const useToast = () => useContext(ToastCtx);

/** Toasts report real operation outcomes only. */
export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<Toast[]>([]);
  const next = useRef(1);
  const push = useCallback((tone: Toast["tone"], text: string) => {
    const id = next.current++;
    setItems((x) => [...x, { id, tone, text }]);
    setTimeout(() => setItems((x) => x.filter((t) => t.id !== id)), tone === "bad" ? 7000 : 3500);
  }, []);
  return (
    <ToastCtx.Provider value={push}>
      {children}
      <div
        className="pointer-events-none fixed bottom-4 right-4 z-[60] space-y-2"
        aria-live="polite"
      >
        {items.map((t) => (
          <div
            key={t.id}
            role="status"
            className={`pointer-events-auto max-w-sm rounded-lg px-4 py-2.5 text-[13px] shadow-lift ${t.tone === "ok" ? "bg-navy text-white" : "bg-danger text-white"}`}
          >
            {t.text}
          </div>
        ))}
      </div>
    </ToastCtx.Provider>
  );
}
