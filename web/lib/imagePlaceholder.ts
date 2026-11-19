/**
 * Blur placeholders for `next/image` (issue #689).
 *
 * `blurDataURL` must be a base64 data URI; these helpers build a small SVG
 * with a blurred slate rectangle so remote images (wallet icons, IPFS assets)
 * show a stable, layout-reserving placeholder while they load.
 */

function toBase64(value: string): string {
  if (typeof window === "undefined") {
    return Buffer.from(value).toString("base64");
  }
  return window.btoa(value);
}

/** A base64 SVG data URI of a blurred `width` x `height` slate block. */
export function blurDataUri(width = 20, height = 20, color = "#1e293b"): string {
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">` +
    `<filter id="b" color-interpolation-filters="sRGB"><feGaussianBlur stdDeviation="${Math.max(
      1,
      Math.round(Math.min(width, height) / 4),
    )}"/></filter>` +
    `<rect width="100%" height="100%" fill="${color}" filter="url(#b)"/></svg>`;
  return `data:image/svg+xml;base64,${toBase64(svg)}`;
}
