import type { DetectedClient, Registry } from "@/lib/types";

const MODES = new Set(["lazy", "grouped", "full"]);

/** How a client discovers tools: its own override, else Auto. Auto is the user's global
 * choice if they made one (mirrors Registry::chosen_global_discovery_mode), else the
 * backend client default, which lists a small catalog in full for clients with a
 * measured crossover. */
export function clientDiscovery(client: DetectedClient, registry: Registry | null) {
  const globalMode = registry?.discoveryMode?.trim().toLowerCase();
  const chosenGlobal =
    (globalMode && MODES.has(globalMode) ? globalMode : "") ||
    (registry?.lazyDiscovery === false ? "full" : "") ||
    (registry?.legacyLazyDiscovery ? "lazy" : "");
  const autoMode = chosenGlobal || client.discovery?.autoMode || "lazy";
  const autoFullMax = chosenGlobal ? null : (client.discovery?.autoFullMaxTools ?? null);
  const storedMode = registry?.clientDiscovery?.[client.id]?.trim().toLowerCase();
  const clientMode = storedMode && MODES.has(storedMode) ? storedMode : "";
  return { autoMode, autoFullMax, clientMode, effectiveMode: clientMode || autoMode };
}
