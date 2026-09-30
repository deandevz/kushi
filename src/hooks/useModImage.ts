import { useState, useEffect } from "react";
import { readModImage } from "../lib/commands";

// Module-level cache so each archive is only unpacked once per session.
const imageCache = new Map<string, Promise<string | null>>();

export function loadModImage(path: string): Promise<string | null> {
  let p = imageCache.get(path);
  if (!p) {
    p = readModImage(path).catch(() => null);
    imageCache.set(path, p);
  }
  return p;
}

export function forgetModImage(path: string) {
  imageCache.delete(path);
}

/** Preview image embedded in a mod archive (data URL), or null if it has none. */
export function useModImage(path: string | null, enabled = true): string | null {
  const [src, setSrc] = useState<string | null>(null);

  useEffect(() => {
    setSrc(null);
    if (!path || !enabled) return;
    let cancelled = false;
    loadModImage(path).then((img) => {
      if (!cancelled) setSrc(img);
    });
    return () => {
      cancelled = true;
    };
  }, [path, enabled]);

  return src;
}
