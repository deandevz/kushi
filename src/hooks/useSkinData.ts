import { useState, useEffect, useCallback } from "react";
import { getCached, getStale, setCache } from "../lib/cache";
import type { Champion, Skin, Chroma, SkinGroup } from "../types";

const SKIN_IDS_URL =
  "https://raw.githubusercontent.com/Alban1911/LeagueSkins/main/resources/en/skin_ids.json";
const TREE_API_URL =
  "https://api.github.com/repos/Alban1911/LeagueSkins/git/trees/main?recursive=1";
const SKINS_JSON_URL =
  "https://raw.communitydragon.org/latest/plugins/rcp-be-lol-game-data/global/default/v1/skins.json";

const SKIN_IDS_CACHE_KEY = "zushi:skin_ids";
// _v3 adds the preview images, needed for skin forms.
const REPO_ZIPS_CACHE_KEY = "zushi:repo_files_v3";
const CHROMAS_CACHE_KEY = "zushi:chromas";

let skinIdsCache: Record<string, string> | null = null;
let repoZipsCache: Map<string, string> | null = null;
let repoImagesCache: Map<string, string> | null = null;
let chromaInfoCache: Map<number, { colors: string[]; parentId: number }> | null = null;

// ---------------------------------------------------------------------------
// Skin forms
//
// Transforming skins (Elementalist Lux, Risen Legend Ahri, Gun Goddess Miss
// Fortune...) change in game only when the server knows you own them, which a
// patched default skin never triggers. The skin repo ships each form as its own
// small mod nested under the base skin, without an entry in skin_ids.json. We
// register them as chroma-like variants so one form can be picked per game.

export interface FormInfo {
  parentId: number;
  image: string | null;
}

// Readable names for forms whose mods don't carry a usable one. Anything not
// listed falls back to "Form N" in repo order (the base skin is form 1).
const FORM_LABELS: Record<string, string> = {
  // Elementalist Lux
  "99991": "Air",
  "99992": "Dark",
  "99993": "Ice",
  "99994": "Magma",
  "99995": "Mystic",
  "99996": "Nature",
  "99997": "Storm",
  "99998": "Water",
  "99999": "Fire",
  // Gun Goddess Miss Fortune
  "21997": "Zero Hour",
  "21998": "Royal Arms",
  "21999": "Starswarm",
  // Revenant Reign Viego
  "234994": "Assassin",
  "234995": "Fighter",
  "234996": "Mage",
  "234997": "Marksman",
  "234998": "Support",
  "234999": "Tank",
  // Risen Legend tiers (CommunityDragon questSkinInfo)
  "103086": "Immortalized Legend",
  "145071": "Immortalized Legend",
  // K/DA ALL OUT Seraphine
  "147002": "Rising Star",
  "147003": "Superstar",
};

const formInfoCache = new Map<number, FormInfo>();
let formsRegisteredFor: [Record<string, string>, Map<string, string>] | null = null;

/** Nested repo entries missing from skin_ids.json become named forms of their parent. */
function registerForms(): void {
  if (!skinIdsCache || !repoZipsCache) return;
  if (formsRegisteredFor?.[0] === skinIdsCache && formsRegisteredFor?.[1] === repoZipsCache) return;

  const byParent = new Map<string, string[]>();
  for (const [id, path] of repoZipsCache) {
    const parts = path.split("/"); // skins/<champ>/<parent>/<id>/<file>
    if (parts.length !== 5 || id in skinIdsCache) continue;
    const parentId = parts[2];
    if (!(parentId in skinIdsCache)) continue;
    byParent.set(parentId, [...(byParent.get(parentId) ?? []), id]);
  }

  for (const [parentId, ids] of byParent) {
    ids.sort((a, b) => parseInt(a, 10) - parseInt(b, 10));
    ids.forEach((id, i) => {
      const label = FORM_LABELS[id] ?? `Form ${i + 2}`;
      skinIdsCache![id] = `${skinIdsCache![parentId]} (${label})`;
      formInfoCache.set(parseInt(id, 10), {
        parentId: parseInt(parentId, 10),
        image: repoImageUrl(id),
      });
    });
  }

  formsRegisteredFor = [skinIdsCache, repoZipsCache];
  indexedFrom = null; // names were added, rebuild the reverse indexes
}

function repoImageUrl(id: string): string | null {
  const path = repoImagesCache?.get(id);
  if (!path) return null;
  const encoded = path.split("/").map((seg) => encodeURIComponent(seg)).join("/");
  return `https://raw.githubusercontent.com/Alban1911/LeagueSkins/main/${encoded}`;
}

export function lookupForm(skinId: number): FormInfo | null {
  return formInfoCache.get(skinId) ?? null;
}

/** Chroma or form: both group under a parent skin as selectable variants. */
function variantOf(id: number): { colors: string[]; parentId: number; image?: string | null } | undefined {
  const form = formInfoCache.get(id);
  if (form) return { colors: [], parentId: form.parentId, image: form.image };
  return chromaInfoCache?.get(id);
}

// Reverse indices over skinIdsCache so name-based lookups are O(1) instead of
// scanning all ~9k entries on every call (My Skins does this per skin, per render).
let skinNameToId: Map<string, string> | null = null;
let skinChampNameToId: Map<string, string> | null = null;
let indexedFrom: Record<string, string> | null = null;

function ensureSkinIndexes(): void {
  if (!skinIdsCache) return;
  if (indexedFrom === skinIdsCache && skinNameToId) return;
  const nameToId = new Map<string, string>();
  const champNameToId = new Map<string, string>();
  for (const [id, name] of Object.entries(skinIdsCache)) {
    if (!nameToId.has(name)) nameToId.set(name, id);
    const champKey = Math.floor(parseInt(id, 10) / 1000);
    const key = `${champKey}:${name}`;
    if (!champNameToId.has(key)) champNameToId.set(key, id);
  }
  skinNameToId = nameToId;
  skinChampNameToId = champNameToId;
  indexedFrom = skinIdsCache;
}

export function ensureSkinIds(): Promise<Record<string, string>> {
  if (skinIdsCache) return Promise.resolve(skinIdsCache);

  const cached = getCached<Record<string, string>>(SKIN_IDS_CACHE_KEY);
  if (cached) {
    skinIdsCache = cached;
    registerForms();
    return Promise.resolve(cached);
  }

  return fetch(SKIN_IDS_URL)
    .then((res) => {
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      return res.json();
    })
    .then((data: Record<string, string>) => {
      setCache(SKIN_IDS_CACHE_KEY, data);
      skinIdsCache = data;
      registerForms();
      return data;
    })
    .catch((err) => {
      const stale = getStale<Record<string, string>>(SKIN_IDS_CACHE_KEY);
      if (stale) {
        skinIdsCache = stale;
        registerForms();
        return stale;
      }
      throw err;
    });
}

export function ensureRepoZips(): Promise<Map<string, string>> {
  if (repoZipsCache) return Promise.resolve(repoZipsCache);

  type RepoCache = { zips: [string, string][]; images: [string, string][] };
  const load = (c: RepoCache) => {
    repoZipsCache = new Map(c.zips);
    repoImagesCache = new Map(c.images);
    registerForms();
    return repoZipsCache;
  };

  const cached = getCached<RepoCache>(REPO_ZIPS_CACHE_KEY);
  if (cached) return Promise.resolve(load(cached));

  return fetch(TREE_API_URL)
    .then((res) => {
      if (!res.ok) throw new Error(`GitHub API ${res.status}`);
      return res.json();
    })
    .then((data: { tree: { path: string; type: string }[] }) => {
      const map = new Map<string, string>();
      const images = new Map<string, string>();
      for (const entry of data.tree) {
        if (entry.type !== "blob" || !entry.path.startsWith("skins/")) continue;
        const png = entry.path.match(/\/(\d+)\.png$/);
        if (png) {
          images.set(png[1], entry.path);
          continue;
        }
        // Paths are id-based, e.g. "skins/222/222020/222025/222025.fantome".
        // The filename stem is the skin id, matching skin_ids.json keys.
        const filename = entry.path.split("/").pop() ?? "";
        const ext = filename.endsWith(".fantome")
          ? ".fantome"
          : filename.endsWith(".zip")
            ? ".zip"
            : null;
        if (!ext) continue;
        const skinId = filename.slice(0, -ext.length);
        if (!/^\d+$/.test(skinId)) continue;
        map.set(skinId, entry.path);
      }
      const fresh: RepoCache = { zips: [...map.entries()], images: [...images.entries()] };
      setCache(REPO_ZIPS_CACHE_KEY, fresh);
      return load(fresh);
    })
    .catch((err) => {
      const stale = getStale<RepoCache>(REPO_ZIPS_CACHE_KEY);
      if (stale) return load(stale);
      throw err;
    });
}

export function ensureChromaInfo(): Promise<Map<number, { colors: string[]; parentId: number }>> {
  if (chromaInfoCache) return Promise.resolve(chromaInfoCache);

  const cached = getCached<[number, { colors: string[]; parentId: number }][]>(CHROMAS_CACHE_KEY);
  if (cached) {
    chromaInfoCache = new Map(cached);
    return Promise.resolve(chromaInfoCache);
  }

  return fetch(SKINS_JSON_URL)
    .then((res) => {
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      return res.json();
    })
    .then((data: Record<string, { id: number; chromas?: { id: number; colors?: string[] }[] }>) => {
      const map = new Map<number, { colors: string[]; parentId: number }>();
      for (const [, skin] of Object.entries(data)) {
        if (!skin.chromas) continue;
        for (const c of skin.chromas) {
          map.set(c.id, { colors: c.colors ?? [], parentId: skin.id });
        }
      }
      chromaInfoCache = map;
      setCache(CHROMAS_CACHE_KEY, [...map.entries()]);
      return map;
    })
    .catch(() => {
      chromaInfoCache = new Map();
      return chromaInfoCache;
    });
}

export function getSkinsForChampion(champion: Champion, skinIds: Record<string, string>): Skin[] {
  const champKey = parseInt(champion.key, 10);
  const baseId = String(champKey * 1000);
  const championRepoName = skinIds[baseId] ?? champion.name;

  const skins: Skin[] = [];
  for (const [id, name] of Object.entries(skinIds)) {
    const numericId = parseInt(id, 10);
    const ownerKey = Math.floor(numericId / 1000);
    if (ownerKey !== champKey) continue;

    const num = numericId % 1000;
    if (num === 0) continue;

    // Skip chromas of the default skin (e.g. "Steel Blitzcrank") — not real,
    // applicable skins and they have no card to group under. See grouping below.
    const chromaInfo = variantOf(numericId);
    if (chromaInfo && chromaInfo.parentId % 1000 === 0) continue;

    skins.push({
      id,
      num,
      name,
      championId: champion.id,
      championName: championRepoName,
    });
  }

  skins.sort((a, b) => a.num - b.num);
  return skins;
}

export function getSkinsGroupedForChampion(
  champion: Champion,
  skinIds: Record<string, string>
): SkinGroup[] {
  const flat = getSkinsForChampion(champion, skinIds);
  if ((!chromaInfoCache || chromaInfoCache.size === 0) && formInfoCache.size === 0) {
    return flat.map((s) => ({ base: s, chromas: [] }));
  }

  const byId = new Map<number, Skin>();
  for (const s of flat) byId.set(parseInt(s.id, 10), s);

  const groups = new Map<number, SkinGroup>();
  const orphans: Chroma[] = [];

  for (const s of flat) {
    const numericId = parseInt(s.id, 10);
    const chromaInfo = variantOf(numericId);

    if (!chromaInfo) {
      groups.set(numericId, { base: s, chromas: [] });
      continue;
    }

    const parentSkin = byId.get(chromaInfo.parentId);
    const parentName = parentSkin?.name ?? s.name.replace(/\s+\([^)]+\)$/, "");
    const chroma: Chroma = {
      ...s,
      parentName,
      colors: chromaInfo.colors,
      image: chromaInfo.image ?? null,
    };

    const parentGroup = groups.get(chromaInfo.parentId);
    if (parentGroup) {
      parentGroup.chromas.push(chroma);
    } else {
      orphans.push(chroma);
    }
  }

  for (const c of orphans) {
    const info = variantOf(parseInt(c.id, 10));
    const g = info ? groups.get(info.parentId) : undefined;
    if (g) {
      g.chromas.push(c);
    } else {
      groups.set(parseInt(c.id, 10), { base: c, chromas: [] });
    }
  }

  const result = [...groups.values()];
  result.sort((a, b) => a.base.num - b.base.num);
  for (const g of result) g.chromas.sort((a, b) => a.num - b.num);
  return result;
}

export function lookupChromaInfo(
  skinName: string
): { colors: string[]; parentName: string; image: string | null } | null {
  if (!skinIdsCache) return null;
  ensureSkinIndexes();

  const foundIdStr = skinNameToId!.get(skinName);
  if (foundIdStr === undefined) return null;

  const info = variantOf(parseInt(foundIdStr, 10));
  if (!info) return null;

  // skinIdsCache is already id→name, so the parent name is a direct lookup.
  const parentName = skinIdsCache[String(info.parentId)] ?? skinName.replace(/\s+\([^)]+\)$/, "");

  return { colors: info.colors, parentName, image: info.image ?? null };
}

/**
 * For a Skin object, return the splash num to use for DDragon images.
 * Chromas don't have their own splash on DDragon - we resolve to the base skin's num.
 */
export function getSplashNum(skin: Skin): number {
  if (!skinIdsCache) return skin.num;

  const match = skin.name.match(/^(.+?)\s+\([^)]+\)$/);
  if (!match) return skin.num;

  ensureSkinIndexes();
  const champKey = Math.floor(parseInt(skin.id, 10) / 1000);
  const baseId = skinChampNameToId!.get(`${champKey}:${match[1]}`);
  return baseId !== undefined ? parseInt(baseId, 10) % 1000 : skin.num;
}

/**
 * For a skin name string, look up the splash num to use for DDragon images.
 * Resolves chromas to their base skin's splash num.
 */
export function lookupSplashNum(skinName: string): number | null {
  if (!skinIdsCache) return null;
  ensureSkinIndexes();

  const foundId = skinNameToId!.get(skinName);
  if (foundId === undefined) return null;

  const champKey = Math.floor(parseInt(foundId, 10) / 1000);

  // Chroma ("BaseName (Variant)") resolves to its base skin's num.
  const match = skinName.match(/^(.+?)\s+\([^)]+\)$/);
  if (match) {
    const baseId = skinChampNameToId!.get(`${champKey}:${match[1]}`);
    if (baseId !== undefined) return parseInt(baseId, 10) % 1000;
  }

  return parseInt(foundId, 10) % 1000;
}

/**
 * Returns the exact download URL for a skin by looking up the repo tree.
 * Returns null if the skin doesn't exist in the repo.
 */
export function skinDownloadUrl(skin: Skin): string | null {
  if (!repoZipsCache) return null;

  const repoPath = repoZipsCache.get(skin.id);
  if (!repoPath) return null;

  const encoded = repoPath
    .split("/")
    .map((seg) => encodeURIComponent(seg))
    .join("/");
  return `https://raw.githubusercontent.com/Alban1911/LeagueSkins/main/${encoded}`;
}

export function isSkinAvailable(skin: Skin): boolean {
  if (!repoZipsCache) return true; // optimistic before tree loads
  return repoZipsCache.has(skin.id);
}

export function useSkinData(champion: Champion | null) {
  const [skinIds, setSkinIds] = useState<Record<string, string> | null>(skinIdsCache);
  const [skins, setSkins] = useState<Skin[]>([]);
  const [groups, setGroups] = useState<SkinGroup[]>([]);
  const [loading, setLoading] = useState(!skinIdsCache || !repoZipsCache);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (skinIdsCache && repoZipsCache) {
      setSkinIds(skinIdsCache);
      setLoading(false);
      void ensureChromaInfo();
      return;
    }

    void ensureChromaInfo();
    Promise.all([ensureSkinIds(), ensureRepoZips()])
      .then(([ids]) => {
        setSkinIds(ids);
      })
      .catch((err) => setError(String(err)))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => {
    if (!champion || !skinIds) {
      setSkins([]);
      setGroups([]);
      return;
    }
    setSkins(getSkinsForChampion(champion, skinIds));
    setGroups(getSkinsGroupedForChampion(champion, skinIds));
  }, [champion, skinIds]);

  const getSkins = useCallback(
    (champ: Champion): Skin[] => {
      if (!skinIds) return [];
      return getSkinsForChampion(champ, skinIds);
    },
    [skinIds]
  );

  return { skins, groups, loading, error, getSkins };
}
