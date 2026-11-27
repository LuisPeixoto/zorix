// The settings of the program that are kept in the database: long conversations, how long runs are kept, telemetry and scanned pages.
// One copy, shared by the cards that edit different parts of it, so saving one card never undoes another.

import { type AppSettings, appSettings, appSettingsSet } from "../api.ts";

class AppSettingsStore {
  /** What is saved, or `null` until it was read. */
  value = $state<AppSettings | null>(null);
  error = $state<unknown>(null);

  async load(): Promise<void> {
    try {
      this.value = await appSettings();
      this.error = null;
    } catch (e) {
      this.error = e;
    }
  }

  /** Save a change made to a copy of what is saved. Throws what the backend refused. */
  async save(change: (settings: AppSettings) => AppSettings): Promise<AppSettings> {
    const current = this.value ?? (await appSettings());
    this.value = await appSettingsSet(change(structuredClone($state.snapshot(current)) as AppSettings));
    return this.value;
  }
}

export const settingsStore = new AppSettingsStore();
