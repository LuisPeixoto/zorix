// What MCP servers ask while tools run. The list is read again each time the backend says it changed.
import { type AskAnswer, type PendingAsk, answerServerAsk, errorText, onServerAsks, serverAsks } from "../api.ts";
import { toast } from "./toast.svelte.ts";

class AsksStore {
  list = $state<PendingAsk[]>([]);
  busy = $state(false);

  async init(): Promise<void> {
    try {
      await onServerAsks(() => void this.refresh());
    } catch {
      /* Outside the app (a browser preview) there is nothing to listen to. */
    }
    await this.refresh();
  }

  async refresh(): Promise<void> {
    try {
      this.list = await serverAsks();
    } catch {
      this.list = [];
    }
  }

  async answer(ask: PendingAsk, answer: AskAnswer): Promise<void> {
    this.busy = true;
    try {
      await answerServerAsk(ask.id, ask.hash, answer);
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      this.busy = false;
      await this.refresh();
    }
  }
}

export const asks = new AsksStore();
