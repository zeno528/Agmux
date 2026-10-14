import { api } from "../api";
import type { AppState } from "../types";

export async function switchProfileFromTray(client: "codex" | "claude", id: string, state: AppState, refresh: () => Promise<void>) {
  if (client === "claude") await api.claudeApplyProfile(id);
  else await api.codexApplyProfile(id);
  await refresh();
  if (client !== "codex" || !state.settings.auto_restart) return;
  await api.restartCodex();
  await refresh();
}
