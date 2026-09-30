import { useState, useEffect, useCallback } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { open } from "@tauri-apps/plugin-dialog";
import { Upload, Package } from "lucide-react";
import CustomCard from "./CustomCard";
import { championAvatar } from "../hooks/useChampions";
import type { Champion, CustomMod } from "../types";

type CustomGroup = { key: string; label: string; champion: Champion | null; mods: CustomMod[] };

// Champion groups alphabetically, global mods (maps, UI, fonts) last.
function groupCustoms(customs: CustomMod[]): CustomGroup[] {
  const byChamp = new Map<string, CustomGroup>();
  const other: CustomMod[] = [];
  for (const c of customs) {
    if (!c.champion) {
      other.push(c);
      continue;
    }
    const g = byChamp.get(c.champion.id) ?? {
      key: c.champion.id,
      label: c.champion.name,
      champion: c.champion,
      mods: [],
    };
    g.mods.push(c);
    byChamp.set(c.champion.id, g);
  }
  const groups = [...byChamp.values()].sort((a, b) => a.label.localeCompare(b.label));
  if (other.length) groups.push({ key: "__other", label: "Other mods", champion: null, mods: other });
  for (const g of groups) g.mods.sort((a, b) => a.displayName.localeCompare(b.displayName));
  return groups;
}

interface CustomsProps {
  customs: CustomMod[];
  patcherActive: boolean;
  onAdd: (path: string, autoEnable: boolean) => Promise<void>;
  onRemove: (name: string) => Promise<void>;
  onToggle: (name: string) => void;
}

export default function Customs({
  customs,
  patcherActive,
  onAdd,
  onRemove,
  onToggle,
}: CustomsProps) {
  const [draggingOver, setDraggingOver] = useState(false);
  const [adding, setAdding] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleAdd = useCallback(
    async (path: string, autoEnable: boolean) => {
      const fileName = path.split("/").pop() ?? path;
      setAdding(fileName);
      setError(null);
      try {
        await onAdd(path, autoEnable);
      } catch (e) {
        setError(String(e));
      } finally {
        setAdding(null);
      }
    },
    [onAdd]
  );

  useEffect(() => {
    let unlisten: (() => void) | null = null;

    getCurrentWebviewWindow()
      .onDragDropEvent((event) => {
        const { type } = event.payload;
        if (type === "drop") {
          setDraggingOver(false);
          if (patcherActive) return;
          const paths = (event.payload as { type: "drop"; paths: string[] }).paths;
          const zips = paths.filter((p) => {
            const lower = p.toLowerCase();
            return lower.endsWith(".zip") || lower.endsWith(".fantome");
          });
          zips.forEach((p) => handleAdd(p, zips.length === 1));
        } else if (type === "leave") {
          setDraggingOver(false);
        } else {
          setDraggingOver(true);
        }
      })
      .then((fn) => {
        unlisten = fn;
      });

    return () => {
      unlisten?.();
    };
  }, [handleAdd, patcherActive]);

  async function handleBrowse() {
    if (patcherActive) return;
    const selected = await open({
      multiple: true,
      filters: [{ name: "Mod", extensions: ["zip", "fantome"] }],
    });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    for (const p of paths) handleAdd(p, paths.length === 1);
  }

  const activeCount = customs.filter((c) => c.enabled).length;

  return (
    <div className="flex h-full flex-col">
      <div className="border-border flex shrink-0 items-center border-b px-4 py-3">
        <span className="text-ink-muted text-sm tabular-nums select-none">
          {customs.length} {customs.length === 1 ? "custom" : "customs"}
          {activeCount > 0 && (
            <span className="text-gold-400 ml-1.5 font-medium">{activeCount} active</span>
          )}
        </span>
      </div>

      <div
        onClick={handleBrowse}
        className={[
          "border-border mx-4 mt-4 flex items-center justify-center gap-2 rounded-lg border-2 border-dashed px-4 transition-all",
          customs.length > 0 ? "flex-row py-2.5" : "flex-col py-5",
          patcherActive
            ? "cursor-default opacity-40"
            : draggingOver
              ? "cursor-pointer border-gold-400 bg-gold-400/10"
              : "hover:border-charcoal-50/30 hover:bg-charcoal-300/30 cursor-pointer",
        ].join(" ")}
      >
        <Upload
          size={18}
          strokeWidth={1.5}
          className={draggingOver && !patcherActive ? "text-gold-400" : "text-ink-muted"}
        />
        <p
          className={`text-sm font-medium ${draggingOver && !patcherActive ? "text-gold-400" : "text-ink-secondary"}`}
        >
          {patcherActive
            ? "Stop the patcher to add mods"
            : adding
              ? `Adding ${adding}...`
              : "Drop mods here"}
        </p>
        {!patcherActive && <p className="text-ink-muted text-xs">{customs.length > 0 ? "· " : ""}or click to browse</p>}
      </div>

      {error && (
        <div className="bg-error/10 text-error mx-4 mt-2 rounded-md px-3 py-2 text-xs">{error}</div>
      )}

      {customs.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center">
          <div className="bg-charcoal-300 flex h-12 w-12 items-center justify-center rounded-full">
            <Package size={20} strokeWidth={1.5} className="text-ink-muted" />
          </div>
          <p className="text-ink-secondary text-sm">No custom mods yet</p>
        </div>
      ) : (
        <div className="flex-1 overflow-y-auto pb-4">
          {groupCustoms(customs).map(({ key, label, champion, mods }) => (
            <div key={key} className="border-border border-b px-4 py-3">
              <div className="mb-2.5 flex items-center gap-2 select-none">
                {champion ? (
                  <img
                    src={championAvatar(champion.id)}
                    alt={label}
                    className="h-6 w-6 rounded object-cover"
                    draggable={false}
                  />
                ) : (
                  <div className="bg-charcoal-300 flex h-6 w-6 items-center justify-center rounded">
                    <Package size={12} strokeWidth={1.5} className="text-ink-muted" />
                  </div>
                )}
                <span className="text-ink text-xs font-medium">{label}</span>
                <span className="text-ink-muted text-[10px] tabular-nums">{mods.length}</span>
                {champion && (
                  <span className="text-ink-muted ml-auto text-[10px]">one active per champion</span>
                )}
              </div>
              <div className="flex flex-wrap gap-3">
                {mods.map((custom) => (
                  <CustomCard
                    key={custom.name}
                    custom={custom}
                    patcherActive={patcherActive}
                    onToggle={onToggle}
                    onRemove={onRemove}
                  />
                ))}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
