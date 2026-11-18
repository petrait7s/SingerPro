import { describe, expect, it } from "vitest";
import {
  bpToPercent,
  formatDhaka,
  formatPaisa,
  paisaToInput,
  parseTaka,
  percentToBp,
} from "./format";

describe("formatPaisa (must mirror Rust Money::Display)", () => {
  it.each([
    [0, "৳0.00"],
    [99_900, "৳999.00"],
    [100_000, "৳1,000.00"],
    [12_500_050, "৳1,25,000.50"],
    [1_234_567_800, "৳1,23,45,678.00"],
    [-5, "-৳0.05"],
  ])("%i -> %s", (p, s) => expect(formatPaisa(p)).toBe(s));
});

describe("formatDhaka", () => {
  it("renders UTC+6", () => {
    expect(formatDhaka("2026-01-01T20:00:00.000Z")).toContain("02 Jan 2026");
  });
});

describe("percent <-> basis points (no floats)", () => {
  it.each([
    ["0", 0],
    ["7.5", 750],
    ["12", 1200],
    ["0.05", 5],
    ["100", 10000],
  ])("%s -> %i", (s, bp) => expect(percentToBp(s)).toBe(bp));
  it.each(["", "abc", "100.01", "101", "1.234", "-1"])("rejects %j", (s) =>
    expect(percentToBp(s)).toBeNull(),
  );
  it.each([
    [0, "0"],
    [750, "7.5"],
    [1200, "12"],
    [5, "0.05"],
  ])("%i -> %s", (bp, s) => expect(bpToPercent(bp)).toBe(s));
});

describe("parseTaka / paisaToInput (exact, no floats)", () => {
  it.each([
    ["1250", 125000],
    ["1,250.5", 125050],
    ["৳ 99.99", 9999],
    ["0.07", 7],
    [".5", 50],
    ["-3.10", -310],
  ])("%s -> %i", (s, p) => expect(parseTaka(s)).toBe(p));
  it.each(["", "abc", "1.234", "1..2", ".", "1e3", "--1"])("rejects %j", (s) =>
    expect(parseTaka(s)).toBeNull(),
  );
  it("round-trips", () => {
    for (const p of [0, 1, 10, 99, 100, 12345, 99999999])
      expect(parseTaka(paisaToInput(p))).toBe(p);
  });
  it("0.1 + 0.2 style traps stay exact", () => {
    expect((parseTaka("0.1") ?? 0) + (parseTaka("0.2") ?? 0)).toBe(parseTaka("0.3"));
  });
});
