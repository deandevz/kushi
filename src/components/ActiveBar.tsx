import type { ReactNode } from "react";
import { X } from "lucide-react";

export interface ActiveItem {
  key: string;
  label: string;
  sublabel: string;
  thumb: ReactNode;
  onRemove: () => void;
}

/** Everything that will be applied, in one strip: official skins and customs. */
export default function ActiveBar({
  items,
  patcherActive,
}: {
  items: ActiveItem[];
  patcherActive: boolean;
}) {
  return (
    <div className="border-border bg-charcoal-500/40 flex h-14 shrink-0 items-center gap-3 border-b px-4">
      <span className="text-ink-muted w-12 shrink-0 text-[10px] font-semibold tracking-wider uppercase select-none">
        Active
      </span>
      {items.length === 0 ? (
        <span className="text-ink-muted text-xs select-none">Nothing selected</span>
      ) : (
        <div className="flex min-w-0 flex-1 gap-2 overflow-x-auto py-0.5">
          {items.map((item) => (
            <div
              key={item.key}
              className="bg-charcoal-400 border-gold-400/30 group flex h-10 shrink-0 items-center gap-2 rounded-md border pr-2"
              title={`${item.label}\n${item.sublabel}`}
            >
              <div className="relative h-10 w-10 shrink-0 overflow-hidden rounded-l-md">
                {item.thumb}
              </div>
              <div className="max-w-36 min-w-0">
                <p className="text-ink truncate text-[11px] leading-tight">{item.label}</p>
                <p className="text-ink-muted truncate text-[9px] leading-tight">{item.sublabel}</p>
              </div>
              {!patcherActive && (
                <button
                  onClick={item.onRemove}
                  className="text-ink-muted hover:text-error cursor-pointer opacity-0 transition-opacity group-hover:opacity-100"
                  title="Deselect"
                >
                  <X size={12} strokeWidth={2.5} />
                </button>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
