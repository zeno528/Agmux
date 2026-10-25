import { ProfileOverwriteRequired } from "../../api";

export async function applyProfileWithConfirmation(apply: (overwrite?: boolean) => Promise<void>, confirmOverwrite: () => Promise<boolean>) {
  try {
    await apply();
  } catch (error) {
    if (!(error instanceof ProfileOverwriteRequired)) throw error;
    if (!await confirmOverwrite()) return false;
    await apply(true);
  }
  return true;
}
