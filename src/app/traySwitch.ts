import { api } from "../api";
import type { AppState } from "../types";

export async function switchProfileFromTray(id: string, state: AppState, refresh: () => Promise<void>) {
  await api.codexApplyProfile(id);
  await refresh();
  if (!state.settings.auto_restart) return;
  await api.restartCodex();
  await refresh();
}
