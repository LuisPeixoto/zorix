// Small Svelte actions shared by the kit.

/** A tooltip that follows the element, shown on hover and keyboard focus. Plain text only. */
export function tooltip(node: HTMLElement, text: string) {
  let label = text;
  let tip: HTMLDivElement | null = null;

  function show() {
    if (!label || tip) return;
    tip = document.createElement("div");
    tip.textContent = label;
    tip.setAttribute("role", "tooltip");
    tip.className =
      "pointer-events-none fixed z-[70] rounded-md bg-fg px-2 py-1 text-xs font-medium text-bg shadow-pop";
    document.body.appendChild(tip);
    const r = node.getBoundingClientRect();
    const t = tip.getBoundingClientRect();
    const left = Math.min(Math.max(8, r.left + r.width / 2 - t.width / 2), window.innerWidth - t.width - 8);
    const below = r.bottom + 6 + t.height < window.innerHeight;
    tip.style.left = `${left}px`;
    tip.style.top = `${below ? r.bottom + 6 : r.top - t.height - 6}px`;
  }
  function hide() {
    tip?.remove();
    tip = null;
  }

  node.addEventListener("mouseenter", show);
  node.addEventListener("mouseleave", hide);
  node.addEventListener("focus", show);
  node.addEventListener("blur", hide);
  node.addEventListener("click", hide);

  return {
    update(next: string) {
      label = next;
      if (tip) tip.textContent = next;
    },
    destroy() {
      hide();
      node.removeEventListener("mouseenter", show);
      node.removeEventListener("mouseleave", hide);
      node.removeEventListener("focus", show);
      node.removeEventListener("blur", hide);
      node.removeEventListener("click", hide);
    },
  };
}

const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** Keep Tab inside a dialog, put focus in it when it opens, and give focus back when it closes. */
export function trapFocus(node: HTMLElement) {
  const before = document.activeElement as HTMLElement | null;
  const items = () => [...node.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.offsetParent !== null);
  queueMicrotask(() => (items()[0] ?? node).focus());

  function onKey(event: KeyboardEvent) {
    if (event.key !== "Tab") return;
    const list = items();
    if (list.length === 0) return event.preventDefault();
    const first = list[0];
    const last = list[list.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }
  node.addEventListener("keydown", onKey);
  return {
    destroy() {
      node.removeEventListener("keydown", onKey);
      before?.focus?.();
    },
  };
}
