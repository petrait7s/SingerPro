/** Display-only helpers. Authoritative money math lives in Rust (integer paisa). */

export function formatPaisa(paisa: number): string {
  const neg = paisa < 0;
  const abs = Math.abs(Math.trunc(paisa));
  const whole = Math.floor(abs / 100).toString();
  const frac = (abs % 100).toString().padStart(2, "0");
  let grouped = whole;
  if (whole.length > 3) {
    let head = whole.slice(0, -3);
    const tail = whole.slice(-3);
    const parts: string[] = [];
    while (head.length > 2) {
      parts.unshift(head.slice(-2));
      head = head.slice(0, -2);
    }
    if (head) parts.unshift(head);
    grouped = `${parts.join(",")},${tail}`;
  }
  return `${neg ? "-" : ""}৳${grouped}.${frac}`;
}

const DHAKA = "Asia/Dhaka";

export function formatDhaka(iso: string): string {
  return new Intl.DateTimeFormat("en-GB", {
    timeZone: DHAKA,
    day: "2-digit",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hour12: true,
  }).format(new Date(iso));
}

/** "7.5" -> 750 basis points, using string math (no floating point). null when invalid. */
export function percentToBp(input: string): number | null {
  const m = /^(\d{1,3})(?:\.(\d{1,2}))?$/.exec(input.trim());
  if (!m) return null;
  const bp = Number(m[1]) * 100 + Number((m[2] ?? "").padEnd(2, "0") || 0);
  return bp <= 10_000 ? bp : null;
}

export function bpToPercent(bp: string | number): string {
  const n = Math.trunc(Number(bp));
  if (!Number.isFinite(n)) return "";
  const whole = Math.floor(n / 100);
  const frac = n % 100;
  return frac === 0 ? String(whole) : `${whole}.${String(frac).padStart(2, "0").replace(/0$/, "")}`;
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}
