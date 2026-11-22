/**
 * Custom `next/image` loader for dynamic images (issue #689).
 *
 * Pass it per-image (`<Image loader={dynamicImageLoader} />`) so only images
 * that need it bypass the default optimizer:
 *   - `ipfs://<cid>/path` is resolved through a public gateway
 *     (`NEXT_PUBLIC_IPFS_GATEWAY`, default https://ipfs.io/ipfs/);
 *   - images served from the configured CDN (`NEXT_PUBLIC_IMAGE_CDN_URL`) get
 *     `w` and `q` query parameters so the CDN can resize them;
 *   - everything else is returned unchanged.
 */

export interface ImageLoaderProps {
  src: string;
  width: number;
  quality?: number;
}

const DEFAULT_IPFS_GATEWAY = "https://ipfs.io/ipfs/";
const DEFAULT_QUALITY = 75;

export function resolveIpfsUrl(src: string, gateway = DEFAULT_IPFS_GATEWAY): string {
  const path = src.replace(/^ipfs:\/\//, "").replace(/^ipfs\//, "");
  return `${gateway.replace(/\/?$/, "/")}${path}`;
}

export function dynamicImageLoader({ src, width, quality }: ImageLoaderProps): string {
  const q = quality ?? DEFAULT_QUALITY;

  if (src.startsWith("ipfs://")) {
    return resolveIpfsUrl(src, process.env.NEXT_PUBLIC_IPFS_GATEWAY || DEFAULT_IPFS_GATEWAY);
  }

  const cdn = process.env.NEXT_PUBLIC_IMAGE_CDN_URL;
  if (cdn && src.startsWith(cdn)) {
    const url = new URL(src);
    url.searchParams.set("w", String(width));
    url.searchParams.set("q", String(q));
    return url.toString();
  }

  return src;
}
