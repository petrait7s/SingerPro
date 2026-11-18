import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";

describe("App shell", () => {
  it("renders navigation and switches pages honestly", () => {
    render(<App />);
    expect(screen.getAllByText("Dashboard").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: /Installments/ }));
    expect(screen.getByText(/scheduled for P4/)).toBeTruthy();
  });

  it("shows an error, not fake data, outside the desktop runtime", async () => {
    render(<App />);
    expect(await screen.findByRole("alert")).toBeTruthy();
  });
});
