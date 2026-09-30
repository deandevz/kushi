import { useState } from "react";
import { FileArchive } from "lucide-react";
import { splashUrl, findChampionByName } from "../hooks/useChampions";
import { lookupSplashNum } from "../hooks/useSkinData";
import { useModImage } from "../hooks/useModImage";
import type { Champion } from "../types";

function Img({ src, alt, onError }: { src: string; alt: string; onError?: () => void }) {
  const [loaded, setLoaded] = useState(false);
  return (
    <>
      {!loaded && (
        <div className="from-charcoal-600 via-charcoal-400 to-charcoal-600 animate-shimmer absolute inset-0 bg-linear-to-r bg-size-[200%_100%]" />
      )}
      <img
        src={src}
        alt={alt}
        className={[
          "h-full w-full object-cover object-[center_20%]",
          loaded ? "opacity-100" : "opacity-0",
        ].join(" ")}
        loading="lazy"
        draggable={false}
        onLoad={() => setLoaded(true)}
        onError={onError}
      />
    </>
  );
}

/**
 * Preview for a mod archive: the image it ships (META/image.png, thumbnail...),
 * else the champion's default splash, else a generic icon.
 */
export function ModThumb({
  path,
  hasImage = true,
  champion,
  alt,
  dim,
  compact,
}: {
  path: string;
  hasImage?: boolean;
  champion: Champion | null | undefined;
  alt: string;
  dim?: boolean;
  /** Small square (Active bar): center the fallback icon, no room reserved for a caption. */
  compact?: boolean;
}) {
  const embedded = useModImage(path, hasImage);
  const [splashFailed, setSplashFailed] = useState(false);

  if (embedded) return <Img key={path} src={embedded} alt={alt} />;

  if (champion && !splashFailed) {
    return (
      <div className={dim ? "h-full w-full opacity-60 grayscale-[35%]" : "h-full w-full"}>
        <Img src={splashUrl(champion.id, 0)} alt={alt} onError={() => setSplashFailed(true)} />
      </div>
    );
  }

  return (
    <div
      className={[
        "bg-charcoal-300 flex h-full w-full items-center justify-center",
        compact ? "" : "pb-4",
      ].join(" ")}
    >
      <FileArchive size={compact ? 18 : 30} strokeWidth={compact ? 1.5 : 1} className="text-ink-muted/60" />
    </div>
  );
}

/** Official skin splash from DDragon; falls back to an image inside the zip. */
export function SkinThumb({
  championName,
  skinName,
  zipPath,
  compact,
}: {
  championName: string;
  skinName: string;
  zipPath?: string;
  compact?: boolean;
}) {
  const [failed, setFailed] = useState(false);
  const champ = findChampionByName(championName);
  const num = lookupSplashNum(skinName);

  if (!champ || num === null || failed) {
    if (zipPath) return <ModThumb path={zipPath} champion={champ} alt={skinName} dim compact={compact} />;
    return <div className="bg-charcoal-600 h-full w-full" />;
  }

  return <Img src={splashUrl(champ.id, num)} alt={skinName} onError={() => setFailed(true)} />;
}
