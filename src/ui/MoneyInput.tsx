import { useState } from "react";
import { paisaToInput, parseTaka } from "../lib/format";
import { Input } from "./controls";

/** Taka text input that reports exact integer paisa (null while invalid/empty). */
export function MoneyInput({
  id,
  value,
  onChange,
  disabled,
  placeholder,
}: {
  id?: string;
  value: number | null;
  onChange: (paisa: number | null) => void;
  disabled?: boolean;
  placeholder?: string;
}) {
  const [text, setText] = useState(value === null ? "" : paisaToInput(value));
  const bad = text.trim() !== "" && parseTaka(text) === null;
  return (
    <Input
      id={id}
      inputMode="decimal"
      value={text}
      disabled={disabled}
      placeholder={placeholder ?? "0.00"}
      aria-invalid={bad}
      className={`text-right tabular-nums ${bad ? "!border-danger" : ""}`}
      onChange={(e) => {
        setText(e.target.value);
        onChange(e.target.value.trim() === "" ? null : parseTaka(e.target.value));
      }}
    />
  );
}
