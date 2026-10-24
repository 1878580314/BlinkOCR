const modifiers = new Set(["Control", "Shift", "Alt", "Meta", "AltGraph"]);
export function recordedShortcut(
  event: Pick<
    KeyboardEvent,
    | "key"
    | "code"
    | "ctrlKey"
    | "altKey"
    | "shiftKey"
    | "metaKey"
    | "repeat"
    | "isComposing"
  >,
): string | null {
  if (
    event.repeat ||
    event.isComposing ||
    modifiers.has(event.key) ||
    event.key === "Escape"
  )
    return null;
  const keys: string[] = [];
  if (event.ctrlKey) keys.push("Ctrl");
  if (event.altKey) keys.push("Alt");
  if (event.shiftKey) keys.push("Shift");
  if (event.metaKey) keys.push("Super");
  const aliases: Record<string, string> = {
    Space: "Space",
    ArrowUp: "ArrowUp",
    ArrowDown: "ArrowDown",
    ArrowLeft: "ArrowLeft",
    ArrowRight: "ArrowRight",
  };
  const key = event.code.startsWith("Key")
    ? event.code.slice(3)
    : event.code.startsWith("Digit")
      ? event.code.slice(5)
      : (aliases[event.code] ?? event.code);
  if (!key || key === "Unidentified") return null;
  keys.push(key);
  return keys.join("+");
}
