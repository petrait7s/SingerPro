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
