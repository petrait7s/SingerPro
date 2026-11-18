import { describe, expect, it } from "vitest";
import { formatDhaka, formatPaisa } from "./format";

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
