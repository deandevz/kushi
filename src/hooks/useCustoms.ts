import { useState, useEffect, useCallback } from "react";
import { importCustom, listCustoms, removeCustom, readModInfo } from "../lib/commands";
import { ensureChampions, findChampionByWad } from "./useChampions";
import { forgetModImage } from "./useModImage";
import type { Champion, CustomEntry, CustomMod } from "../types";

const ENABLED_KEY = "zushi_customs_enabled";

function loadEnabled(): Set<string> {
  try {
    const raw = localStorage.getItem(ENABLED_KEY);
    if (raw) return new Set(JSON.parse(raw) as string[]);
  } catch {}
  return new Set();
}

function saveEnabled(enabled: Set<string>) {
  localStorage.setItem(ENABLED_KEY, JSON.stringify([...enabled]));
}

// A mod counts as a champion skin only when every WAD it ships belongs to the
// same champion. Anything else (maps, UI, fonts, multi-champion packs) is global.
function resolveChampion(wads: string[]): Champion | null {
  if (wads.length === 0) return null;
  const champs = wads.map(findChampionByWad);
  const first = champs[0];
  if (!first || champs.some((c) => c?.id !== first.id)) return null;
  return first;
}

async function describe(entry: CustomEntry): Promise<Omit<CustomMod, "enabled">> {
  try {
    const info = await readModInfo(entry.file_path);
    return {
      ...entry,
      displayName: info.name ?? entry.name,
      author: info.author && info.author.toLowerCase() !== "unknown" ? info.author : null,
      hasImage: info.has_image,
      champion: resolveChampion(info.wads),
    };
  } catch {
    return { ...entry, displayName: entry.name, author: null, hasImage: false, champion: null };
  }
}

export function useCustoms() {
  const [customs, setCustoms] = useState<CustomMod[]>([]);
  const [enabled, setEnabled] = useState<Set<string>>(loadEnabled);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      await ensureChampions().catch(() => {});
      const list = await Promise.all((await listCustoms()).map(describe));
      const enabledSet = loadEnabled();
      setCustoms(list.map((e) => ({ ...e, enabled: enabledSet.has(e.name) })));
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // Keep customs list in sync whenever enabled changes
  useEffect(() => {
    setCustoms((prev) => prev.map((c) => ({ ...c, enabled: enabled.has(c.name) })));
  }, [enabled]);

  const updateEnabled = useCallback((fn: (prev: Set<string>) => Set<string>) => {
    setEnabled((prev) => {
      const next = fn(new Set(prev));
      saveEnabled(next);
      return next;
    });
  }, []);

  const addCustom = useCallback(
    async (srcPath: string): Promise<CustomMod> => {
      const fileName = srcPath.split("/").pop() ?? srcPath;
      const name = fileName.replace(/\.(zip|fantome)$/i, "");
      setError(null);
      const entry = await importCustom(srcPath, name);
      forgetModImage(entry.file_path);
      const mod = { ...(await describe(entry)), enabled: false };
      // Show it immediately so it can't be applied before the refresh lists it.
      setCustoms((prev) =>
        [...prev.filter((c) => c.name !== mod.name), mod].sort((a, b) =>
          a.name.localeCompare(b.name)
        )
      );
      return mod;
    },
    []
  );

  const remove = useCallback(
    async (name: string) => {
      await removeCustom(name);
      updateEnabled((next) => {
        next.delete(name);
        return next;
      });
      await refresh();
    },
    [refresh, updateEnabled]
  );

  const clearError = useCallback(() => setError(null), []);

  // Derive from the `enabled` Set (source of truth), not the mirrored
  // `c.enabled` which can be one render behind — otherwise a just-enabled
  // custom can be missing from the paths sent to apply.
  const enabledPaths = customs.filter((c) => enabled.has(c.name)).map((c) => c.file_path);
  const enabledCount = enabledPaths.length;

  return {
    customs,
    enabled,
    updateEnabled,
    addCustom,
    remove,
    refresh,
    enabledPaths,
    enabledCount,
    error,
    clearError,
  };
}
