import { useEffect, useRef } from "react";

import { isMacLike } from "../lib/platform";

export interface Shortcut {
  key: string;
  /** Command on macOS, Control elsewhere. */
  mod?: boolean;
  shift?: boolean;
}

function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

export function useShortcut(shortcut: Shortcut, handler: () => void, enabled = true): void {
  const handlerRef = useRef(handler);
  useEffect(() => {
    handlerRef.current = handler;
  });

  const { key, mod = false, shift = false } = shortcut;

  useEffect(() => {
    if (!enabled) return;
    const onKeyDown = (event: KeyboardEvent) => {
      const modPressed = isMacLike() ? event.metaKey : event.ctrlKey;
      if (modPressed !== mod || event.shiftKey !== shift || event.altKey) return;
      if (event.key.toLowerCase() !== key.toLowerCase()) return;
      if (!mod && isEditable(event.target)) return;
      // Window-level shortcuts must not act on the page hidden behind a modal.
      if (document.querySelector("dialog[open]")) return;
      event.preventDefault();
      handlerRef.current();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [key, mod, shift, enabled]);
}

/** Human label for tooltips: "⌘R" on macOS, "Ctrl+R" elsewhere. */
export function shortcutLabel({ key, mod, shift }: Shortcut): string {
  const mac = isMacLike();
  const parts: string[] = [];
  if (mod) parts.push(mac ? "⌘" : "Ctrl");
  if (shift) parts.push(mac ? "⇧" : "Shift");
  parts.push(key.length === 1 ? key.toUpperCase() : key);
  return parts.join(mac ? "" : "+");
}

/** Value for `aria-keyshortcuts`. */
export function ariaShortcut({ key, mod, shift }: Shortcut): string {
  const parts: string[] = [];
  if (mod) parts.push(isMacLike() ? "Meta" : "Control");
  if (shift) parts.push("Shift");
  parts.push(key.length === 1 ? key.toUpperCase() : key);
  return parts.join("+");
}
