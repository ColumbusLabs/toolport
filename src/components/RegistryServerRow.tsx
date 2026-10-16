import { msg, Trans, translate, useI18n, type SourceMessage } from "@/lib/i18n";
import { referenceProvider } from "@/lib/secretRefs";
import { toastError } from "@/lib/toast";
import { isPersonalSync } from "@/lib/personalSync";
import { invoke } from "@tauri-apps/api/core";
import {
  lazy,
  Suspense,
  useEffect,
  useId,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import {
  ChevronDown,
  Copy,
  RefreshCw,
  KeyRound,
  LogIn,
  Pencil,
  Trash2,
  Users,
} from "lucide-react";
import { isDownloadLauncher } from "@/lib/launcher";
import { serverFailureLabel } from "@/lib/serverHealth";
import { errorHeadline, shortenUrls } from "@/lib/errors";
import { toast } from "sonner";
import type { ProbeResult, Registry, ServerEntry } from "@/lib/types";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { LazySecretsDialog as SecretsDialog } from "@/components/LazySecretsDialog";
import { LazyServerDialog as ServerDialog } from "@/components/LazyServerDialog";
import { LaunchSetupDialog } from "@/components/LaunchSetupDialog";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ServerLogo } from "@/components/ServerLogo";

const ServerToolsPanel = lazy(() =>
  import("@/components/ServerToolsPanel").then((m) => ({ default: m.ServerToolsPanel })),
);

interface Props {
  openTools?: boolean;
  server: ServerEntry;
  registry: Registry | null;
  enabled: boolean;
  busy?: boolean;
  health?: ProbeResult;
  onToggle: (enabled: boolean) => void;
  onRemove: () => void;
  onRegistryChange: (registry: Registry) => void;
  /** Re-run the health probe (e.g. after authenticating). */
  onReprobe?: () => void;
}

type Status = "disabled" | "checking" | "connected" | "needs-auth" | "error";

function statusOf(enabled: boolean, health?: ProbeResult): Status {
  if (!enabled) return "disabled";
  // No probe result yet (loading, or queued behind an in-flight probe): checking.
  if (!health) return "checking";
  if (health.ok) return "connected";
  if (health.authRequired) return "needs-auth";
  return "error";
}

const STATUS_TEXT: Record<Status, string> = {
  disabled: "text-muted-foreground",
  checking: "text-muted-foreground",
  connected: "text-muted-foreground",
  "needs-auth": "text-warning",
  error: "text-destructive",
};

const STATUS_ARIA_LABEL: Record<Status, SourceMessage> = {
  disabled: msg("Server disabled"),
  checking: msg("Checking connection"),
  connected: msg("Connected"),
  "needs-auth": msg("Authentication required"),
  error: msg("Connection error"),
};

const DETAIL_TABS = ["overview", "tools"] as const;
type DetailTab = (typeof DETAIL_TABS)[number];

const PANEL =
  "flex flex-col gap-2.5 rounded-md focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring";

const ACTION =
  "inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring";

export function RegistryServerRow({
  registry,
  server,
  enabled,
  busy,
  health,
  onToggle,
  onRemove,
  onRegistryChange,
  onReprobe,
  openTools,
}: Props) {
  const { t } = useI18n();
  const [expanded, setExpanded] = useState(false);
  const [detailTab, setDetailTab] = useState<DetailTab>("overview");
  // Tab and panel IDs are per row, so two expanded rows never share them.
  const detailId = useId();
  const tabId = (tab: DetailTab) => `${detailId}-${tab}-tab`;
  const panelId = (tab: DetailTab) => `${detailId}-${tab}-panel`;
  const tabRefs = useRef<Partial<Record<DetailTab, HTMLButtonElement | null>>>({});
  // One tab stop for the tablist: arrows move between tabs (wrapping), Home and
  // End jump to the ends, and selection follows focus.
  const onTabKeyDown = (event: KeyboardEvent<HTMLButtonElement>, tab: DetailTab) => {
    const count = DETAIL_TABS.length;
    const index = DETAIL_TABS.indexOf(tab);
    const targets: Partial<Record<string, number>> = {
      ArrowRight: (index + 1) % count,
      ArrowLeft: (index - 1 + count) % count,
      Home: 0,
      End: count - 1,
    };
    const nextIndex = targets[event.key];
    if (nextIndex === undefined) return;
    event.preventDefault();
    const next = DETAIL_TABS[nextIndex];
    setDetailTab(next);
    tabRefs.current[next]?.focus();
  };
  useEffect(() => {
    if (openTools) {
      // eslint-disable-next-line react-hooks/set-state-in-effect -- Open the requested tools tab when navigation selects this row.
      setExpanded(true);
      setDetailTab("tools");
    }
  }, [openTools]);

  const target =
    server.command !== null
      ? [server.command, ...server.args].join(" ")
      : (server.url ?? "");
  const providers = [
    ...new Set(
      [...server.env, ...(server.headerKeys ?? []), ...(server.launch?.inputs ?? [])]
        .map((e) => referenceProvider(e.source?.ref)?.name)
        .filter(Boolean),
    ),
  ];
  const secretCount = server.env.filter((e) => e.secret).length;
  const status = statusOf(enabled, health);
  const requiredLaunch = server.launch?.inputs.filter((input) => input.required) ?? [];
  const missingPlainLaunch = requiredLaunch.filter(
    (input) => !input.secret && !input.value?.trim(),
  );
  // A probe that sits in "checking" past a few seconds is still starting. Name
  // that wait so a slow initialize reads as progress rather than a missing route.
  // Download launchers keep the more specific "Installing…" label.
  const launcher = isDownloadLauncher(server.command, server.args);
  const [initializing, setInitializing] = useState(false);
  // Reset during render (not in the effect) so a later re-check starts back at
  // "Checking…" instead of flashing a stale "Installing…".
  if (initializing && status !== "checking") setInitializing(false);
  useEffect(() => {
    if (status !== "checking") return;
    const t = setTimeout(() => setInitializing(true), 4000);
    return () => clearTimeout(t);
  }, [status]);
  // Team-synced servers are tagged `team:<id>`. An admin manages them centrally, so a
  // member sees a Team badge and can't edit/remove them locally (a local change would
  // just re-sync away), but still authenticates them (keys stay on their own machine).
  const personalSync = isPersonalSync(registry);
  const isTeam = (server.source?.startsWith("team:") ?? false) && !personalSync;

  const label =
    status === "needs-auth"
      ? serverFailureLabel(health!)
      : status === "connected"
        ? translate(
            health?.toolCount === 1
              ? msg("Ready · {count} tool")
              : msg("Ready · {count} tools"),
            { count: health?.toolCount ?? 0 },
          )
        : status === "error"
          ? serverFailureLabel(health!)
          : status === "checking"
            ? initializing
              ? launcher
                ? translate("Installing…")
                : translate("Initializing…")
              : translate("Checking…")
            : requiredLaunch.length
              ? missingPlainLaunch.length
                ? translate("Setup required: {value0}", {
                    value0: missingPlainLaunch.map((input) => input.label).join(", "),
                  })
                : translate("Disabled · check launch setup")
              : translate("Disabled");

  // Next free "Name (N)" for the duplicate-for-another-account action.
  const existingNames = new Set(registry?.servers.map((s) => s.name.toLowerCase()) ?? []);
  const baseName = server.name.replace(/\s\(\d+\)$/, "");
  let duplicateName = `${baseName} (2)`;
  let index = 2;
  while (existingNames.has(duplicateName.toLowerCase())) {
    index++;
    duplicateName = `${baseName} (${index})`;
  }

  const stop = (e: { stopPropagation: () => void }) => e.stopPropagation();

  return (
    <div
      className={`border-b border-border/60 last:border-b-0 ${enabled ? "" : "opacity-60"}`}
    >
      {/* Mouse users can click anywhere on the row to expand. Keyboard and screen
          reader users use the chevron button at the end. The row itself is not a
          button, so the toggle and Authenticate controls aren't nested inside one. */}
      <div
        onClick={() => setExpanded((v) => !v)}
        className="flex cursor-pointer items-center gap-3 px-1 py-3 transition-colors hover:bg-accent/40"
      >
        <span className="flex items-center" onClick={stop}>
          <Switch
            checked={enabled}
            disabled={busy}
            onCheckedChange={onToggle}
            aria-label={t("Toggle {value1}", { value1: server.name })}
          />
        </span>

        <ServerLogo name={server.name} transport={server.transport} size={28} />

        <span title={server.name} className="min-w-0 flex-1 truncate text-sm font-[550]">
          {server.name}
          {providers.length > 0 && (
            <span className="ml-2 text-[10px] font-normal text-muted-foreground">
              {t("Keys from {providers}", { providers: providers.join(", ") })}
            </span>
          )}
        </span>

        {isTeam ? (
          <span className="hidden shrink-0 items-center gap-1 rounded bg-info/15 px-1.5 py-0.5 text-[11px] font-medium text-info md:inline-flex">
            <Trans
              message={msg("{value1}Team")}
              values={{ value1: <Users className="size-3" aria-hidden="true" /> }}
            />
          </span>
        ) : (
          <span className="hidden max-w-40 shrink-0 truncate rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground md:inline">
            {personalSync
              ? server.syncLocalOnly
                ? translate("This machine only")
                : registry?.team?.personalSyncState?.conflicts?.[
                      server.teamOriginalId ?? server.id
                    ]
                  ? translate("Sync conflict")
                  : server.teamEnableReview === true
                    ? translate("Needs review")
                    : translate("Synced")
              : translate("Personal")}
            {server.source?.startsWith("imported:")
              ? ` · ${t("from {client}", { client: server.source.slice("imported:".length) })}`
              : ""}
          </span>
        )}

        <span className="ml-auto flex shrink-0 items-center gap-2.5">
          <span
            role="status"
            aria-label={
              initializing
                ? launcher
                  ? translate("Installing the server package")
                  : translate("Server initializing")
                : status === "needs-auth" || status === "error"
                  ? label
                  : status === "connected"
                    ? label.replace(" · ", ", ")
                    : t(STATUS_ARIA_LABEL[status])
            }
            className="sr-only"
          />
          {status === "needs-auth" ? (
            <>
              <StatusLabel status={status} label={label} error={health?.error ?? null} />
              <SecretsDialog
                server={server}
                onSaved={onRegistryChange}
                onChanged={onReprobe}
                trigger={
                  <button
                    onClick={stop}
                    className="inline-flex items-center gap-1.5 rounded-md border border-warning/40 px-2.5 py-1 text-xs text-warning transition-colors hover:bg-warning/10 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-warning"
                  >
                    <LogIn className="size-3.5" />
                    {health?.authTarget === "service_credential"
                      ? translate("Edit service key")
                      : translate("Sign in")}
                  </button>
                }
              />
            </>
          ) : (
            <StatusLabel status={status} label={label} error={health?.error ?? null} />
          )}

          {health && !health.ok && health.error && (
            <span className="flex gap-1">
              {status === "error" && onReprobe && (
                <button
                  type="button"
                  title={translate("Retry {value0} connection", { value0: server.name })}
                  className={ACTION}
                  disabled={busy}
                  onClick={(e) => {
                    stop(e);
                    onReprobe();
                  }}
                >
                  <RefreshCw className="size-3.5" aria-hidden="true" />
                  {translate("Retry")}
                </button>
              )}
              <button
                type="button"
                title={translate("View the connection error and server output")}
                className={ACTION}
                onClick={(e) => {
                  stop(e);
                  setExpanded(true);
                  setDetailTab("overview");
                }}
              >
                {translate("View log")}
              </button>
            </span>
          )}
          <button
            type="button"
            title={
              expanded
                ? translate("Hide server details")
                : translate("Show server details")
            }
            onClick={(e) => {
              e.stopPropagation();
              setExpanded((v) => !v);
            }}
            aria-expanded={expanded}
            aria-label={
              expanded
                ? t("Hide {value1} details", { value1: server.name })
                : t("Show {value1} details", { value1: server.name })
            }
            className="rounded p-0.5 text-muted-foreground/50 transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          >
            <ChevronDown
              className={`size-4 transition-transform ${expanded ? "rotate-180" : ""}`}
              aria-hidden="true"
            />
          </button>
        </span>
      </div>

      {!enabled && server.teamRouteRemoved && (
        <p className="px-3.5 pb-2 pl-12 text-xs text-muted-foreground">
          {translate("Removed or disabled by the team. Your personal server stays off.")}
        </p>
      )}

      {expanded && (
        <div className="flex flex-col gap-2.5 px-3.5 pt-0.5 pb-3 pl-12">
          <div
            role="tablist"
            aria-label={translate("{value0} details", { value0: server.name })}
            className="flex gap-1 border-b pb-2"
          >
            {DETAIL_TABS.map((tab) => (
              <button
                key={tab}
                ref={(node) => {
                  tabRefs.current[tab] = node;
                }}
                role="tab"
                id={tabId(tab)}
                aria-selected={detailTab === tab}
                aria-controls={panelId(tab)}
                tabIndex={detailTab === tab ? 0 : -1}
                className={ACTION}
                onClick={() => setDetailTab(tab)}
                onKeyDown={(event) => onTabKeyDown(event, tab)}
              >
                {tab === "overview" ? translate("Overview") : translate("Tools")}
              </button>
            ))}
          </div>
          {/* The inactive panel stays as an empty, hidden placeholder so every
              tab's aria-controls resolves. Its content mounts only when selected. */}
          {DETAIL_TABS.filter((tab) => tab !== detailTab).map((tab) => (
            <div
              key={tab}
              role="tabpanel"
              id={panelId(tab)}
              aria-labelledby={tabId(tab)}
              hidden
            />
          ))}
          {detailTab === "tools" ? (
            <div
              role="tabpanel"
              id={panelId("tools")}
              aria-labelledby={tabId("tools")}
              tabIndex={0}
              className={PANEL}
            >
              <Suspense fallback={<p>{translate("Loading tools…")}</p>}>
                <ServerToolsPanel
                  serverId={server.id}
                  registry={registry}
                  onRegistryChange={onRegistryChange}
                />
              </Suspense>
            </div>
          ) : (
            <div
              role="tabpanel"
              id={panelId("overview")}
              aria-labelledby={tabId("overview")}
              tabIndex={0}
              className={PANEL}
            >
              {!!requiredLaunch.length && (
                <p className="text-xs text-muted-foreground">
                  {t(
                    "Launch setup: {inputs}. Open Launch setup to add or review these values before enabling.",
                    { inputs: requiredLaunch.map((input) => input.label).join(", ") },
                  )}
                </p>
              )}
              {target && (
                <code className="block rounded-md bg-muted px-2 py-1.5 font-mono text-xs break-all text-muted-foreground">
                  {target}
                </code>
              )}
              {health && !health.ok && health.error && (
                <div className="flex flex-col gap-1">
                  {/* Lead with a readable headline so the useful signal (exit status,
                  EADDRINUSE, a 401) isn't buried under a stack trace + a giant
                  OAuth URL; the full output stays below, bounded and scrollable
                  with long URLs shortened. */}
                  <div className="flex items-start justify-between gap-2">
                    <p className="text-xs font-medium text-warning">
                      {errorHeadline(health.error)}
                    </p>
                    <button
                      type="button"
                      onClick={(e) => {
                        e.stopPropagation();
                        void navigator.clipboard.writeText(health?.error ?? "");
                        toast.success(translate("Error copied"));
                      }}
                      title={translate("Copy the full error")}
                      className="inline-flex shrink-0 items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                    >
                      <Copy className="size-3" />
                      {translate("Copy")}
                    </button>
                  </div>
                  <p className="max-h-32 overflow-y-auto font-mono text-[11px] break-words whitespace-pre-wrap text-muted-foreground">
                    {shortenUrls(health.error)}
                  </p>
                </div>
              )}

              <div className="flex flex-wrap items-center gap-1">
                <SecretsDialog
                  server={server}
                  onSaved={onRegistryChange}
                  onChanged={onReprobe}
                  trigger={
                    <button className={ACTION}>
                      <KeyRound className="size-3.5" />
                      {translate("Secrets")}
                      {secretCount > 0 ? ` (${secretCount})` : ""}
                    </button>
                  }
                />

                {!!server.launch?.inputs.length && (
                  <LaunchSetupDialog
                    server={server}
                    onSaved={onRegistryChange}
                    onChanged={onReprobe}
                    trigger={
                      <button className={ACTION}>{translate("Launch setup")}</button>
                    }
                  />
                )}

                {personalSync && (
                  <label className="inline-flex items-center gap-2 text-xs">
                    <input
                      type="checkbox"
                      checked={server.syncLocalOnly === true}
                      onChange={async (e) => {
                        try {
                          const r = await invoke<Registry>("personal_sync_local_only", {
                            serverId: server.id,
                            localOnly: e.target.checked,
                          });
                          onRegistryChange(r);
                        } catch (error) {
                          toastError(String(error));
                        }
                      }}
                    />{" "}
                    {translate("Keep on this machine only")}
                    <span className="text-xs text-muted-foreground">
                      {translate("Copies already on your other machines stay there.")}
                    </span>
                  </label>
                )}
                {personalSync && (
                  <span className="text-xs text-muted-foreground">
                    {server.syncLocalOnly
                      ? translate("This machine only")
                      : translate("Synced")}
                  </span>
                )}
                {isTeam ? (
                  <span className="inline-flex items-center gap-1.5 px-2 py-1 text-xs text-muted-foreground">
                    <Users className="size-3.5" aria-hidden="true" />
                    {translate(
                      "Managed by your team. Ask an admin to change or remove it.",
                    )}
                  </span>
                ) : (
                  <>
                    <ServerDialog
                      onSaved={onRegistryChange}
                      initial={{ ...server, name: duplicateName }}
                      existingNames={registry?.servers.map((s) => s.name) ?? []}
                      trigger={
                        <button
                          className={ACTION}
                          title={translate("Add another account")}
                        >
                          <Copy className="size-3.5" />
                          {translate("Duplicate")}
                        </button>
                      }
                    />

                    <ServerDialog
                      onSaved={onRegistryChange}
                      editId={server.id}
                      initial={server}
                      existingNames={registry?.servers.map((s) => s.name) ?? []}
                      trigger={
                        <button className={ACTION}>
                          <Pencil className="size-3.5" />
                          {translate("Edit")}
                        </button>
                      }
                    />

                    <ConfirmDialog
                      trigger={
                        <button
                          disabled={busy}
                          className={`${ACTION} hover:bg-destructive/10 hover:text-destructive`}
                        >
                          <Trash2 className="size-3.5" />
                          {translate("Remove")}
                        </button>
                      }
                      title={translate("Remove {value0}?", { value0: server.name })}
                      description={
                        personalSync && !server.syncLocalOnly
                          ? translate(
                              "This removes the server from Toolport and your other machines. Any saved secrets stay in your keychain.",
                            )
                          : translate(
                              "This deletes the server from Toolport on this machine. Any saved secrets stay in your keychain.",
                            )
                      }
                      confirmLabel={translate("Remove")}
                      destructive
                      onConfirm={onRemove}
                    />
                  </>
                )}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function StatusLabel({
  status,
  label,
  error,
}: {
  status: Status;
  label: string;
  error: string | null;
}) {
  const text = (
    <span
      aria-hidden="true"
      className={`inline-flex items-center gap-1.5 text-[13px] whitespace-nowrap ${STATUS_TEXT[status]}`}
    >
      <span
        className={`size-[7px] shrink-0 rounded-full ${status === "connected" ? "bg-success" : status === "needs-auth" ? "bg-warning" : status === "error" ? "bg-destructive" : "bg-muted-foreground/60"}`}
      />
      {label}
    </span>
  );
  if (status === "error" && error) {
    return (
      <Tooltip>
        <TooltipTrigger asChild>{text}</TooltipTrigger>
        <TooltipContent side="top" className="max-w-xs">
          <p className="text-xs text-warning">{errorHeadline(error)}</p>
        </TooltipContent>
      </Tooltip>
    );
  }
  return text;
}
