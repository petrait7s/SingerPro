import { describe, expect, it } from "vitest";
import { bpToPercent, formatDhaka, formatPaisa, percentToBp } from "./format";

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
