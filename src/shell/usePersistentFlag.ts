import { useCallback, useState } from "react";

function read(key: string): boolean {
  try {
    return localStorage.getItem(key) === "true";
  } catch {
    return false;
  }
}

export function usePersistentFlag(key: string): [boolean, (value: boolean) => void] {
  const [value, setValue] = useState(() => read(key));

  const update = useCallback(
    (next: boolean) => {
      setValue(next);
      try {
        localStorage.setItem(key, String(next));
      } catch {
        // Storage blocked: keep the in-memory value for this session.
      }
    },
    [key],
  );

  return [value, update];
}
