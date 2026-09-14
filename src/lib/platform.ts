export function isMacLike(): boolean {
  return /Mac|iPhone|iPad/.test(navigator.userAgent);
}
