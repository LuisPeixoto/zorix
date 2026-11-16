// Short messages that fade by themselves. Use them for results of actions ("Key saved"), not for things that need an answer.

export type ToastKind = "ok" | "err" | "info";
export interface Toast {
  id: number;
  kind: ToastKind;
  text: string;
}

class ToastStore {
  items = $state<Toast[]>([]);
  #next = 1;

  /** Show a message. Errors stay longer. Returns the id so it can be dismissed. */
  show(kind: ToastKind, text: string, ms = kind === "err" ? 7000 : 3500): number {
    const id = this.#next++;
    this.items = [...this.items.slice(-3), { id, kind, text }];
    if (ms > 0) setTimeout(() => this.dismiss(id), ms);
    return id;
  }
  ok(text: string): number {
    return this.show("ok", text);
  }
  err(text: string): number {
    return this.show("err", text);
  }
  info(text: string): number {
    return this.show("info", text);
  }
  dismiss(id: number): void {
    this.items = this.items.filter((t) => t.id !== id);
  }
}

export const toast = new ToastStore();
