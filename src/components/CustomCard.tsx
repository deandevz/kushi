import { Check, X } from "lucide-react";
import { ModThumb } from "./Thumbs";
import type { CustomMod } from "../types";

interface CustomCardProps {
  custom: CustomMod;
  patcherActive: boolean;
  onToggle: (name: string) => void;
  onRemove?: (name: string) => void;
  size?: "sm" | "md";
}

export default function CustomCard({
  custom,
  patcherActive,
  onToggle,
  onRemove,
  size = "md",
}: CustomCardProps) {
  const title = [custom.displayName, custom.author && `by ${custom.author}`, custom.name]
    .filter(Boolean)
    .join("\n");

  return (
    <div className="relative">
      <button
        onClick={() => !patcherActive && onToggle(custom.name)}
        disabled={patcherActive}
        className={[
          "relative overflow-hidden rounded transition-all",
          size === "sm" ? "h-24 w-32" : "h-28 w-40",
          patcherActive ? "cursor-default opacity-50" : "cursor-pointer",
          custom.enabled
            ? "ring-gold-400 ring-offset-charcoal-400 ring-2 ring-offset-1"
            : patcherActive
              ? "ring-charcoal-50/20 ring-1"
              : "ring-charcoal-50/20 hover:ring-charcoal-50/40 ring-1",
        ].join(" ")}
        title={title}
      >
        <ModThumb
          path={custom.file_path}
          hasImage={custom.hasImage}
          champion={custom.champion}
          alt={custom.displayName}
          dim
        />

        {!onRemove && (
          <span className="bg-charcoal-600/85 text-gold-400 absolute top-1 left-1 rounded-sm px-1 py-px text-[8px] font-semibold tracking-wider uppercase">
            Custom
          </span>
        )}

        {custom.enabled && (
          <div className="bg-gold-400 absolute top-1.5 right-1.5 flex h-5 w-5 items-center justify-center rounded-full border border-white/30 shadow-sm">
            <Check size={11} strokeWidth={3} className="text-charcoal-600" />
          </div>
        )}

        <div className="absolute inset-x-0 bottom-0 bg-linear-to-t from-black/85 via-black/45 to-transparent px-2 pt-5 pb-1.5 text-left">
          <p className="line-clamp-2 text-[11px] leading-tight text-white/95">
            {custom.displayName}
          </p>
          {custom.author && size === "md" && (
            <p className="truncate text-[9px] text-white/55">by {custom.author}</p>
          )}
        </div>
      </button>

      {onRemove && !patcherActive && (
        <button
          onClick={() => onRemove(custom.name)}
          className="bg-charcoal-500 hover:bg-error text-ink-muted absolute top-1.5 left-1.5 flex h-5 w-5 cursor-pointer items-center justify-center rounded-full shadow transition-colors hover:text-white"
          title="Remove"
        >
          <X size={10} strokeWidth={2.5} />
        </button>
      )}
    </div>
  );
}
