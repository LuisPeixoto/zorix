import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";

import { attachmentsCheck, errorText } from "../api.ts";
import { type AttachedFile, MAX_ATTACHED, addChecked, estimatedChars, withOutsideFlags } from "../attach.ts";
import { t } from "../i18n/index.ts";
import { toast } from "./toast.svelte.ts";

/** The files one form is giving its run (ADR-0049). The program checks each one as it is chosen and again when the run starts. */
export class AttachedFiles {
  files = $state<AttachedFile[]>([]);
  /** Files are being dragged over the window. */
  dropping = $state(false);
  private readonly workspaceOf: () => string | undefined;

  constructor(workspaceOf: () => string | undefined) {
    this.workspaceOf = workspaceOf;
  }

  get paths(): string[] {
    return this.files.map((f) => f.path);
  }

  get bytes(): number {
    return this.files.reduce((sum, f) => sum + f.bytes, 0);
  }

  get chars(): number {
    return estimatedChars(this.files);
  }

  /** Check what was chosen and add what can be attached. A refusal is said with its reason; the rest is kept. */
  async add(paths: string[]): Promise<void> {
    if (paths.length === 0) return;
    try {
      const checked = await attachmentsCheck(paths, this.workspaceOf());
      const next = addChecked(this.files, checked);
      this.files = next.files;
      for (const refused of next.refused) toast.err(refused.problem);
      if (next.overLimit) toast.err(t("attach.atMostFilesCan", { count: MAX_ATTACHED }));
    } catch (e) {
      toast.err(errorText(e));
    }
  }

  remove(path: string): void {
    this.files = this.files.filter((f) => f.path !== path);
  }

  clear(): void {
    this.files = [];
  }

  /** Ask the program again which files are outside the workspace, after the workspace changed. */
  async recheck(): Promise<void> {
    if (this.files.length === 0) return;
    try {
      const checked = await attachmentsCheck(this.paths, this.workspaceOf());
      // Only stored when something changed: a form that reacts to its files must not be woken by a check that found nothing new.
      const next = withOutsideFlags(this.files, checked);
      if (next !== this.files) this.files = next;
    } catch {
      // The flags stay as they were; the run checks everything again when it starts.
    }
  }

  /** The system file dialog. */
  async choose(): Promise<void> {
    try {
      const chosen = await open({ multiple: true, title: t("attach.chooseFilesToGive") });
      if (Array.isArray(chosen)) await this.add(chosen);
      else if (typeof chosen === "string") await this.add([chosen]);
    } catch (e) {
      toast.err(errorText(e));
    }
  }

  /** Follow files dropped on the window. Returns a function that stops listening (outside the app window there is nothing to follow). */
  async listen(): Promise<() => void> {
    try {
      return await getCurrentWebview().onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "enter" || payload.type === "over") this.dropping = true;
        else if (payload.type === "leave") this.dropping = false;
        else if (payload.type === "drop") {
          this.dropping = false;
          void this.add(payload.paths);
        }
      });
    } catch {
      return () => {};
    }
  }
}
