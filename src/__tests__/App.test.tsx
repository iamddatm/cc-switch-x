import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import App from "../App";

test("renders all sidebar entries and switches panes", () => {
  render(<App />);
  for (const label of ["MCP", "Skills", "Prompts", "Rules", "Subagents", "Hooks", "Sessions", "Settings"]) {
    expect(screen.getByRole("button", { name: label })).toBeInTheDocument();
  }
  fireEvent.click(screen.getByRole("button", { name: "Rules" }));
  expect(screen.getByRole("heading", { name: "Rules" })).toBeInTheDocument();
});
