import { msg, Trans, translate, useI18n } from "@/lib/i18n";
import { AddClientLocationDialog } from "@/components/AddClientLocationDialog";
import { Button } from "@/components/ui/button";
import { trustedClientName, clientIdentityTooltip } from "@/lib/clientIdentity";
import { getClientSessions } from "@/lib/api";
import type { ClientActivity } from "@/lib/types";
import { fmtAgo } from "@/lib/utils";
import { useEffect, useId, useState } from "react";
import { ChevronDown, ChevronRight, Download, MonitorCog, Puzzle } from "lucide-react";
import { ClientLogo } from "@/components/ClientLogo";
import { Badge } from "@/components/ui/badge";
import { EmptyState } from "@/components/ui/empty-state";
import { SectionHeader } from "@/components/ui/section-header";
import { Skeleton } from "@/components/ui/skeleton";
import { clientDiscovery } from "@/lib/clientDiscovery";
import {
  importableServers,
  isGatewayDetected,
  type DetectedClient,
  type Registry,
} from "@/lib/types";

type ClientStatus = "connected" | "ready" | "customized" | "error" | "missing";

function statusOf(client: DetectedClient): ClientStatus {
  if (client.error) return "error";
  if (client.entryState === "customized") return "customized";
  if (client.gatewayInstalled) return "connected";
  if (client.appPresent) return "ready";
  return "missing";
}

function sortClients(clients: DetectedClient[]): DetectedClient[] {
  const rank = (client: DetectedClient) => {
    switch (statusOf(client)) {
      case "connected":
        return 0;
      case "ready":
        return 1;
      case "customized":
        return 2;
      case "error":
        return 3;
      case "missing":
        return 4;
    }
  };
  return [...clients].sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name));
}

function StatusBadge({ status }: { status: ClientStatus }) {
  const { t } = useI18n();
  switch (status) {
    case "connected":
      return <Badge variant="success">{t("Connected")}</Badge>;
    case "ready":
      return <Badge variant="info">{t("Ready to connect")}</Badge>;
    case "customized":
      return <Badge variant="warning">{t("Custom configuration")}</Badge>;
    case "error":
      return <Badge variant="warning">{t("Couldn't read config")}</Badge>;
    case "missing":
      return <Badge variant="outline">{t("Not installed")}</Badge>;
  }
}

/** The short discovery summary for a connected client, and the full sentence. */
function discoverySummary(client: DetectedClient, registry: Registry | null) {
  const { autoFullMax, clientMode, effectiveMode } = clientDiscovery(client, registry);
  if (!clientMode && autoFullMax) {
    const text = translate("Auto: {mode}", {
      mode: translate("full up to {count} tools, then lazy", { count: autoFullMax }),
    });
    return { text, full: text };
  }
  switch (effectiveMode) {
    case "full":
      return {
        text: translate("Full tool list"),
        full: translate(
          "Full tool list. Client per-tool permission rules need Full mode.",
        ),
      };
    case "grouped":
      return {
        text: translate("Browse a server, then call tools"),
        full: translate(
          "Browse a server, then call tools. Client per-tool permission rules need Full mode.",
        ),
      };
    default:
      return {
        text: translate("Search, then call tools"),
        full: translate(
          "Search, then call tools. Client per-tool permission rules need Full mode.",
        ),
      };
  }
}

/** The one-line detail under a client's name, and the fuller text for its tooltip.
 * Matches the GTK Clients page (linux_native/client_rows.rs). */
function clientDetail(
  client: DetectedClient,
  registry: Registry | null,
  asProfile: boolean,
): { text: string; full: string } {
  const status = statusOf(client);
  if (status === "missing")
    return {
      text: translate("Install this client to configure it with Toolport"),
      full: translate("Install this client to configure it with Toolport"),
    };
  if (status === "error") {
    const text = translate("Configuration could not be read safely");
    return { text, full: client.error ? `${text}\n${client.error}` : text };
  }
  const parts: string[] = [];
  const profile = asProfile ? client.claudeProfile : undefined;
  if (profile) {
    // A profile without a known command is already named by its folder.
    if (profile.command) parts.push(profile.folder);
    if (profile.lastUsedMs)
      parts.push(
        translate("Last active {value0}", { value0: fmtAgo(profile.lastUsedMs) }),
      );
  }
  let discovery: string | null = null;
  if (status === "connected") {
    const others =
      client.servers.filter((server) => !isGatewayDetected(server)).length +
      client.pluginServers.length;
    if (!client.usesConnectors && others > 0)
      parts.push(
        translate(
          others === 1 ? msg("{value0} other server") : msg("{value0} other servers"),
          { value0: others },
        ),
      );
    const summary = discoverySummary(client, registry);
    parts.push(summary.text);
    discovery = summary.full;
  } else if (client.usesConnectors) {
    parts.push(translate("Uses account connectors"));
  } else if (!profile) {
    const count = client.servers.length + client.pluginServers.length;
    parts.push(
      count === 0
        ? translate("No local MCP servers")
        : translate(
            count === 1
              ? msg("{value0} local MCP server")
              : msg("{value0} local MCP servers"),
            { value0: count },
          ),
    );
  }
  const text = parts.join(" · ");
  const full = [text, client.configPath, discovery].filter(Boolean).join("\n");
  return { text, full };
}

function ClientRow({
  client,
  registry,
  importCount,
  onSelect,
  variant = "line",
}: {
  client: DetectedClient;
  registry: Registry | null;
  importCount: number;
  onSelect: () => void;
  /** A full-width line, a tile in the Available grid, or one profile in the Claude Code group. */
  variant?: "line" | "tile" | "profile";
}) {
  const { t } = useI18n();
  const status = statusOf(client);
  const profile = variant === "profile" ? client.claudeProfile : undefined;
  const detail = clientDetail(client, registry, variant === "profile");

  return (
    <button
      type="button"
      onClick={onSelect}
      className={`grid min-h-14 w-full items-center gap-3 text-left transition-colors hover:bg-accent/40 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring ${
        profile
          ? "grid-cols-[minmax(0,1fr)_16px] border-b border-border/60 py-2.5 pr-1 pl-11 last:border-b-0 sm:grid-cols-[minmax(0,1fr)_auto_16px]"
          : "grid-cols-[28px_minmax(0,1fr)_16px] sm:grid-cols-[28px_minmax(0,1fr)_auto_16px]"
      } ${
        variant === "tile"
          ? "rounded-lg border border-border/70 px-3 py-2.5"
          : variant === "line"
            ? "border-b border-border/60 px-1 py-2.5 last:border-b-0"
            : ""
      } ${status === "missing" ? "opacity-60" : ""}`}
    >
      {!profile && <ClientLogo id={client.id} name={client.name} size={28} />}
      <span className="min-w-0">
        <span className="flex min-w-0 items-center gap-1.5">
          {profile ? (
            <span className="truncate font-mono text-sm font-medium">
              {profile.command ?? profile.folder}
            </span>
          ) : (
            <span className="break-words text-sm font-medium sm:truncate">
              {client.name}
            </span>
          )}
          {/* An unreadable profile has no account to show, signed in or not. */}
          {profile && status !== "error" && (
            <Badge variant="secondary" className="min-w-0 max-w-64 font-normal">
              <span className="truncate">
                {profile.account ?? t("No claude.ai sign-in")}
              </span>
            </Badge>
          )}
          {client.usesConnectors && (
            <Puzzle
              className="size-3.5 shrink-0 text-info"
              aria-label={t("Uses connectors")}
            />
          )}
        </span>
        <span
          className="block truncate text-xs text-muted-foreground"
          title={detail.full}
        >
          {detail.text}
        </span>
      </span>
      <span
        className={`${profile ? "col-start-1" : "col-start-2"} row-start-2 flex min-w-0 flex-wrap items-center gap-2 sm:col-auto sm:row-auto`}
      >
        {importCount > 0 && status !== "error" && status !== "missing" && (
          <Badge variant="owned">
            <Trans
              message={msg("{value1}{value2} to import")}
              values={{ value1: <Download />, value2: importCount }}
            />
          </Badge>
        )}
        <StatusBadge status={status} />
      </span>
      <ChevronRight
        className={`${profile ? "col-start-2" : "col-start-3"} row-start-1 size-4 text-muted-foreground/50 sm:col-auto sm:row-auto`}
        aria-hidden="true"
      />
    </button>
  );
}

/** A product name, not translated. */
const CLAUDE_CODE = "Claude Code";

/** Whether the group is open and its copies shown. Kept across visits to a client's
 * detail page and moves between sections, as the GTK page keeps it. */
const claudeGroupView = { open: true, copiesShown: false };

/** Claude Code's profiles under one entry. Copies of the default profile (same account
 * in `~/.claude`, read only through CLAUDE_CONFIG_DIR) sit behind a note. */
function ClaudeGroup({
  profiles,
  registry,
  onSelectClient,
}: {
  profiles: DetectedClient[];
  registry: Registry | null;
  onSelectClient: (id: string) => void;
}) {
  const { t } = useI18n();
  const [open, setOpenState] = useState(() => claudeGroupView.open);
  const [copiesShown, setCopiesShownState] = useState(() => claudeGroupView.copiesShown);
  const setOpen = (value: boolean) => {
    claudeGroupView.open = value;
    setOpenState(value);
  };
  const setCopiesShown = (value: boolean) => {
    claudeGroupView.copiesShown = value;
    setCopiesShownState(value);
  };
  const summaryId = useId();
  const copies = profiles.filter((client) => client.claudeProfile?.sameAs);
  // Profiles started by a command first, by command; the rest by folder.
  const shown = profiles
    .filter((client) => !client.claudeProfile?.sameAs)
    .sort((a, b) => {
      const left = a.claudeProfile;
      const right = b.claudeProfile;
      return (
        Number(!left?.command) - Number(!right?.command) ||
        (left?.command ?? left?.folder ?? "").localeCompare(
          right?.command ?? right?.folder ?? "",
        )
      );
    });
  const connected = shown.filter((client) => statusOf(client) === "connected").length;
  const copyOf = copies
    .map((client) => shown.find((other) => other.id === client.claudeProfile?.sameAs))
    .find(Boolean)?.claudeProfile?.command;
  const row = (client: DetectedClient) => (
    <ClientRow
      key={client.id}
      client={client}
      registry={registry}
      importCount={importableServers(client, registry).length}
      onSelect={() => onSelectClient(client.id)}
      variant="profile"
    />
  );
  const copiesNote = copies.length > 0 && (
    <div className="border-t border-border/60">
      <div className="flex items-center gap-3 py-2.5 pr-1 pl-11">
        <p className="min-w-0 flex-1 text-xs text-muted-foreground">
          {t(
            "Also found {folder}: the same account as {command}. Claude Code only reads it when CLAUDE_CONFIG_DIR points there, as some background tools do.",
            {
              folder: copies.map((client) => client.claudeProfile?.folder).join(", "),
              command: copyOf ?? "claude",
            },
          )}
        </p>
        <Button
          variant="ghost"
          size="sm"
          aria-expanded={copiesShown}
          onClick={() => setCopiesShown(!copiesShown)}
        >
          {copiesShown ? t("Hide") : t("Show")}
        </Button>
      </div>
      {copiesShown && <div className="border-t border-border/60">{copies.map(row)}</div>}
    </div>
  );
  if (shown.length === 1) {
    // One real profile and its copies: an ordinary row, with the note under it.
    return (
      <div className="border-b border-border/60 last:border-b-0">
        <ClientRow
          client={shown[0]}
          registry={registry}
          importCount={importableServers(shown[0], registry).length}
          onSelect={() => onSelectClient(shown[0].id)}
        />
        {copiesNote && <div className="bg-muted/20">{copiesNote}</div>}
      </div>
    );
  }
  return (
    <div className="border-b border-border/60 last:border-b-0">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        aria-label={t("Claude Code profiles")}
        aria-describedby={summaryId}
        className="grid min-h-14 w-full grid-cols-[28px_minmax(0,1fr)_16px] items-center gap-3 px-1 py-2.5 text-left transition-colors hover:bg-accent/40 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring"
      >
        <ClientLogo id="claude-code" name={CLAUDE_CODE} size={28} />
        <span className="min-w-0">
          <span className="flex items-center gap-1.5">
            <span className="truncate text-sm font-medium">{CLAUDE_CODE}</span>
            <Badge variant="secondary" className="font-normal">
              {t(
                shown.length === 1 ? msg("{value0} profile") : msg("{value0} profiles"),
                {
                  value0: shown.length,
                },
              )}
            </Badge>
          </span>
          <span id={summaryId} className="block truncate text-xs text-muted-foreground">
            {t("{connected} of {total} connected", {
              connected,
              total: shown.length,
            })}
          </span>
        </span>
        <ChevronDown
          className={`size-4 text-muted-foreground/70 transition-transform ${open ? "" : "-rotate-90"}`}
          aria-hidden="true"
        />
      </button>
      {open && (
        <div className="border-t border-border/60 bg-muted/20">
          {shown.map(row)}
          {copiesNote}
        </div>
      )}
    </div>
  );
}

export function ClientsView({
  clients,
  registry,
  loading = false,
  onSelectClient,
  onChanged,
}: {
  clients: DetectedClient[];
  registry: Registry | null;
  loading?: boolean;
  onSelectClient: (id: string) => void;
  onChanged?: () => Promise<void>;
}) {
  const { t, locale } = useI18n();
  const [addingLocation, setAddingLocation] = useState(false);
  const [showMissing, setShowMissing] = useState(false);
  const [showRecent, setShowRecent] = useState(false);
  const [sessions, setSessions] = useState<ClientActivity[]>([]);
  const [sessionError, setSessionError] = useState(false);
  useEffect(() => {
    let current = true;
    getClientSessions()
      .then((rows) => {
        if (current) {
          setSessions(rows);
          setSessionError(false);
        }
      })
      .catch(() => {
        if (current) setSessionError(true);
      });
    return () => {
      current = false;
    };
  }, [clients]);
  const sorted = sortClients(clients);
  const present = sorted.filter((client) => statusOf(client) !== "missing");
  const missing = sorted.filter((client) => statusOf(client) === "missing");
  const connected = present.filter((client) => statusOf(client) === "connected");
  const available = present.filter((client) => statusOf(client) !== "connected");
  // Several Claude Code profiles share one entry; it sits with the connected clients
  // when any of its profiles is connected.
  const claude = present.filter((client) => client.claudeProfile);
  const claudeShown = claude.filter((client) => !client.claudeProfile?.sameAs);
  const claudeConnected = claudeShown.some((client) => statusOf(client) === "connected");
  const connectedRows = connected.filter((client) => !client.claudeProfile);
  const availableRows = available.filter((client) => !client.claudeProfile);
  const availableCount =
    availableRows.length + (claudeConnected ? 0 : claudeShown.length);
  const claudeGroup = claude.length > 0 && (
    <ClaudeGroup profiles={claude} registry={registry} onSelectClient={onSelectClient} />
  );

  if (loading && clients.length === 0) {
    return (
      <div className="flex flex-col gap-2" aria-label={t("Loading AI clients")}>
        {Array.from({ length: 4 }).map((_, i) => (
          <Skeleton key={i} className="h-14 w-full rounded-lg" />
        ))}
      </div>
    );
  }

  if (!onChanged && clients.length === 0 && sessions.length === 0 && !sessionError) {
    return (
      <EmptyState
        icon={<MonitorCog />}
        title={t("No AI clients detected")}
        description={t(
          "Install Claude Desktop, Cursor, VS Code, or another supported client, then refresh.",
        )}
      />
    );
  }

  return (
    <div className="mx-auto flex w-full max-w-[1080px] flex-col gap-5">
      {onChanged && (
        <>
          <Button
            variant="outline"
            className="self-start"
            onClick={() => setAddingLocation(true)}
          >
            {translate("Add client location")}
          </Button>
          <AddClientLocationDialog
            open={addingLocation}
            onOpenChange={setAddingLocation}
            onAdded={async (id) => {
              await onChanged();
              onSelectClient(id);
            }}
          />
        </>
      )}
      {sessionError && (
        <p role="status" className="text-xs text-muted-foreground">
          {translate("Client session history could not be read.")}
        </p>
      )}
      {present.length === 0 && (
        <EmptyState
          icon={<MonitorCog />}
          title={t("No supported clients installed")}
          description={t(
            "Install Claude Desktop, Cursor, VS Code, or another supported client, then refresh.",
          )}
          className="py-12"
        />
      )}

      {connected.length > 0 && (
        <section>
          <SectionHeader count={connected.length}>
            {translate("Connected to Toolport")}
          </SectionHeader>
          <div className="toolport-list">
            {claudeConnected && claudeGroup}
            {connectedRows.map((client) => (
              <ClientRow
                key={client.id}
                client={client}
                registry={registry}
                importCount={importableServers(client, registry).length}
                onSelect={() => onSelectClient(client.id)}
              />
            ))}
          </div>
        </section>
      )}

      {availableCount > 0 && (
        <section className="@container">
          <SectionHeader count={availableCount}>
            {t("Available to connect")}
          </SectionHeader>
          <p className="mb-2 text-xs text-muted-foreground">
            {t("Select an installed client to connect it or review its configuration.")}
          </p>
          {!claudeConnected && claudeGroup && (
            <div className="toolport-list mb-2.5">{claudeGroup}</div>
          )}
          {availableRows.length > 0 && (
            <div className="grid gap-2.5 @2xl:grid-cols-2">
              {availableRows.map((client) => (
                <ClientRow
                  key={client.id}
                  client={client}
                  registry={registry}
                  importCount={importableServers(client, registry).length}
                  onSelect={() => onSelectClient(client.id)}
                  variant="tile"
                />
              ))}
            </div>
          )}
        </section>
      )}

      {missing.length > 0 && (
        <section>
          <button
            type="button"
            onClick={() => setShowMissing((value) => !value)}
            aria-expanded={showMissing}
            className="flex w-full items-center gap-2 rounded-md px-1 py-1.5 text-left text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          >
            <ChevronRight
              className={`size-3.5 transition-transform ${showMissing ? "rotate-90" : ""}`}
              aria-hidden="true"
            />
            <span className="font-medium">{t("Not installed")}</span>
            <span>{missing.length}</span>
          </button>
          {showMissing && (
            <div className="mt-2 toolport-list">
              {missing.map((client) => (
                <ClientRow
                  key={client.id}
                  client={client}
                  registry={registry}
                  importCount={0}
                  onSelect={() => onSelectClient(client.id)}
                />
              ))}
            </div>
          )}
        </section>
      )}
      {sessions.length > 0 && (
        <section aria-label={translate("Recent client activity")}>
          <button
            type="button"
            onClick={() => setShowRecent((value) => !value)}
            aria-expanded={showRecent}
            className="flex w-full items-center gap-2 rounded-md px-1 py-1.5 text-left text-xs text-muted-foreground hover:text-foreground"
          >
            <ChevronRight
              className={`size-3.5 ${showRecent ? "rotate-90" : ""}`}
              aria-hidden="true"
            />
            {translate("Recent client activity")}
            <span>{sessions.length}</span>
          </button>
          {showRecent && (
            <div className="mt-2 toolport-list">
              {sessions.map((client) => (
                <div
                  key={trustedClientName(client)}
                  className="border-b border-border/60 px-1 py-3 last:border-b-0"
                >
                  <p
                    className="truncate text-sm font-medium"
                    title={
                      clientIdentityTooltip(client) ??
                      [
                        client.clientLabel &&
                          translate("Reports itself as: {value0}", {
                            value0: client.clientLabel,
                          }),
                        translate("{value0} recorded sessions", {
                          value0: client.sessionCount ?? 1,
                        }),
                      ]
                        .filter(Boolean)
                        .join(" · ")
                    }
                  >
                    {trustedClientName(client)}
                  </p>
                  <p
                    className="text-xs text-muted-foreground"
                    title={translate(
                      "Calls today counts retained tool calls since local midnight. Older calls may no longer be retained.",
                    )}
                  >
                    {client.lastActiveMs
                      ? translate("Last active {value0}", {
                          value0: fmtAgo(client.lastActiveMs),
                        })
                      : translate("Last active time unavailable")}
                    {client.callsToday != null &&
                      translate(
                        client.callsToday === 1
                          ? msg(" · {count} call today")
                          : msg(" · {count} calls today"),
                        { count: client.callsToday.toLocaleString(locale) },
                      )}
                    {client.firstCatalogSize != null &&
                      translate(" · Last saw {value0} tools", {
                        value0: client.firstCatalogSize.toLocaleString(locale),
                      })}
                  </p>
                </div>
              ))}
            </div>
          )}
        </section>
      )}
    </div>
  );
}
