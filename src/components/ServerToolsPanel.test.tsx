import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Registry } from "@/lib/types";
import { ServerToolsPanel } from "./ServerToolsPanel";

const api = vi.hoisted(() => ({
  listServerTools: vi.fn(),
  callTool: vi.fn(),
  setToolEnabled: vi.fn(),
  listServerResources: vi.fn(),
  listServerPrompts: vi.fn(),
  readResource: vi.fn(),
  getPrompt: vi.fn(),
}));
vi.mock("@/lib/api", () => api);

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}
vi.mock("@/lib/toast", () => ({ toastError: vi.fn() }));
const registry = {
  servers: [{ id: "alpha", name: "Alpha", disabledTools: ["remove"] }],
  profiles: [{ id: "local", name: "Local", enabledServerIds: ["alpha"] }],
  activeProfileId: "local",
} as Registry;
const tools = [
  {
    name: "read",
    toolportQuarantine: "clear",
    description: "Read a record",
    annotations: { readOnlyHint: true },
    inputSchema: {
      type: "object",
      required: ["id"],
      properties: { id: { type: "integer" } },
    },
  },
  {
    name: "remove",
    toolportQuarantine: "quarantined",
    description: "Remove a record",
    annotations: { destructiveHint: true },
    inputSchema: { type: "object" },
  },
];

describe("server Tools panel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.listServerTools.mockResolvedValue(tools);
    api.callTool.mockResolvedValue({ content: [{ type: "text", text: "Record found" }] });
    api.listServerResources.mockResolvedValue([]);
    api.listServerPrompts.mockResolvedValue([]);
  });

  it("loads only its server and shows hints, disabled tools and quarantine", async () => {
    render(
      <ServerToolsPanel
        serverId="alpha"
        registry={registry}
        onRegistryChange={vi.fn()}
      />,
    );
    await screen.findByText("read");
    expect(api.listServerTools).toHaveBeenCalledWith("alpha");
    expect(screen.getByText("read-only")).toBeVisible();
    expect(screen.getByText("destructive")).toBeVisible();
    expect(screen.getByText("Disabled")).toBeVisible();
    expect(screen.getByText("Quarantined")).toBeVisible();
    expect(screen.queryByText("Pick a server…")).not.toBeInTheDocument();
  });

  it("validates schema arguments, calls the selected server and renders the result", async () => {
    render(
      <ServerToolsPanel
        serverId="alpha"
        registry={registry}
        onRegistryChange={vi.fn()}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: /Read a record/ }));
    fireEvent.click(screen.getByRole("button", { name: "Call tool" }));
    expect(api.callTool).not.toHaveBeenCalled();
    expect(screen.getByText(/Fill in required field/)).toBeVisible();
    fireEvent.change(screen.getByLabelText(/id/), { target: { value: "42" } });
    fireEvent.click(screen.getByRole("button", { name: "Call tool" }));
    await waitFor(() =>
      expect(api.callTool).toHaveBeenCalledWith("alpha", "read", { id: 42 }),
    );
    expect(await screen.findByText("Record found")).toBeVisible();
  });

  it("refreshes quarantine state when the default access changes", async () => {
    const view = render(
      <ServerToolsPanel
        serverId="alpha"
        registry={registry}
        onRegistryChange={vi.fn()}
      />,
    );
    await screen.findByText("Quarantined");
    api.listServerTools.mockResolvedValue(
      tools.map((tool) => ({ ...tool, toolportQuarantine: "clear" })),
    );
    view.rerender(
      <ServerToolsPanel
        serverId="alpha"
        registry={{ ...registry, defaultAccessProfileId: "other" }}
        onRegistryChange={vi.fn()}
      />,
    );
    await waitFor(() =>
      expect(screen.queryByText("Quarantined")).not.toBeInTheDocument(),
    );
    expect(screen.getAllByText("Not quarantined")).toHaveLength(2);
  });

  it("reports unknown quarantine when the retained state cannot be read", async () => {
    api.listServerTools.mockResolvedValue(
      tools.map((tool) => ({ ...tool, toolportQuarantine: "unknown" })),
    );
    render(
      <ServerToolsPanel
        serverId="alpha"
        registry={registry}
        onRegistryChange={vi.fn()}
      />,
    );
    await screen.findByText("read");
    expect(screen.getAllByText("Quarantine unknown")).toHaveLength(2);
  });

  it("retries a failed resources list in place with the same server id", async () => {
    api.listServerResources
      .mockRejectedValueOnce(new Error("resources down"))
      .mockResolvedValueOnce([
        { uri: "file:///notes.txt", name: "notes.txt", mimeType: "text/plain" },
      ]);
    render(
      <ServerToolsPanel
        serverId="alpha"
        registry={registry}
        onRegistryChange={vi.fn()}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Resources" }));
    expect(await screen.findByText("Error: resources down")).toBeVisible();
    expect(api.listServerResources).toHaveBeenCalledTimes(1);
    expect(api.listServerResources).toHaveBeenCalledWith("alpha");
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("notes.txt")).toBeVisible();
    expect(screen.queryByText("Error: resources down")).not.toBeInTheDocument();
    expect(api.listServerResources).toHaveBeenCalledTimes(2);
    expect(api.listServerResources).toHaveBeenNthCalledWith(2, "alpha");
  });

  it("does not duplicate a resources list request when Retry is clicked twice", async () => {
    api.listServerResources
      .mockRejectedValueOnce(new Error("resources down"))
      .mockResolvedValueOnce([
        { uri: "file:///notes.txt", name: "notes.txt", mimeType: "text/plain" },
      ]);
    render(
      <ServerToolsPanel
        serverId="alpha"
        registry={registry}
        onRegistryChange={vi.fn()}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Resources" }));
    const retry = await screen.findByRole("button", { name: "Retry" });
    fireEvent.click(retry);
    fireEvent.click(retry);
    expect(await screen.findByText("notes.txt")).toBeVisible();
    expect(api.listServerResources).toHaveBeenCalledTimes(2);
  });

  it("ignores a late resources list response after the panel unmounts", async () => {
    const pending = deferred<Array<{ uri: string; name: string }>>();
    api.listServerResources.mockReturnValueOnce(pending.promise);
    const view = render(
      <ServerToolsPanel
        serverId="alpha"
        registry={registry}
        onRegistryChange={vi.fn()}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Resources" }));
    expect(api.listServerResources).toHaveBeenCalledWith("alpha");
    view.unmount();
    pending.resolve([{ uri: "file:///late.txt", name: "late.txt" }]);
    await Promise.resolve();
    expect(screen.queryByText("late.txt")).not.toBeInTheDocument();
  });

  it("retries a failed prompts list in place with the same server id", async () => {
    api.listServerPrompts
      .mockRejectedValueOnce(new Error("prompts down"))
      .mockResolvedValueOnce([{ name: "summarize", description: "Summarize text" }]);
    render(
      <ServerToolsPanel
        serverId="alpha"
        registry={registry}
        onRegistryChange={vi.fn()}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Prompts" }));
    expect(await screen.findByText("Error: prompts down")).toBeVisible();
    expect(api.listServerPrompts).toHaveBeenCalledTimes(1);
    expect(api.listServerPrompts).toHaveBeenCalledWith("alpha");
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("summarize")).toBeVisible();
    expect(screen.queryByText("Error: prompts down")).not.toBeInTheDocument();
    expect(api.listServerPrompts).toHaveBeenCalledTimes(2);
    expect(api.listServerPrompts).toHaveBeenNthCalledWith(2, "alpha");
  });

  it("does not duplicate a prompts list request when Retry is clicked twice", async () => {
    api.listServerPrompts
      .mockRejectedValueOnce(new Error("prompts down"))
      .mockResolvedValueOnce([{ name: "summarize", description: "Summarize text" }]);
    render(
      <ServerToolsPanel
        serverId="alpha"
        registry={registry}
        onRegistryChange={vi.fn()}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Prompts" }));
    const retry = await screen.findByRole("button", { name: "Retry" });
    fireEvent.click(retry);
    fireEvent.click(retry);
    expect(await screen.findByText("summarize")).toBeVisible();
    expect(api.listServerPrompts).toHaveBeenCalledTimes(2);
  });

  it("ignores a late prompts list response after the panel unmounts", async () => {
    const pending = deferred<Array<{ name: string }>>();
    api.listServerPrompts.mockReturnValueOnce(pending.promise);
    const view = render(
      <ServerToolsPanel
        serverId="alpha"
        registry={registry}
        onRegistryChange={vi.fn()}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Prompts" }));
    expect(api.listServerPrompts).toHaveBeenCalledWith("alpha");
    view.unmount();
    pending.resolve([{ name: "late-prompt" }]);
    await Promise.resolve();
    expect(screen.queryByText("late-prompt")).not.toBeInTheDocument();
  });
});
