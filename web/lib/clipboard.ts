/**
 * Clipboard helper for full-value copy actions (issue #844 / WEB-43).
 *
 * The value passed here is always the complete, unshortened ID. Shortened
 * labels are presentation only and must never reach the clipboard.
 */

export interface ClipboardWriter {
  writeText(value: string): Promise<void> | void;
}

export type CopyResult = "copied" | "manual";

/**
 * Try the async Clipboard API, then the legacy `execCommand` path. Returns
 * `"manual"` when both fail (for example when permission was denied), so the
 * caller can reveal the full value and ask the user to copy it themselves.
 */
export async function copyValue(
  value: string,
  writer: ClipboardWriter | null | undefined,
  legacyCopy?: (value: string) => boolean,
): Promise<CopyResult> {
  if (writer) {
    try {
      await writer.writeText(value);
      return "copied";
    } catch {
      // Permission denied or insecure context: fall through to the legacy path.
    }
  }
  if (legacyCopy?.(value)) return "copied";
  return "manual";
}

/** Selects a temporary off-screen textarea and copies its contents. */
export function legacyCopyText(value: string, doc: Document): boolean {
  const textarea = doc.createElement("textarea");
  textarea.value = value;
  textarea.setAttribute("readonly", "");
  textarea.style.position = "fixed";
  textarea.style.opacity = "0";
  doc.body.appendChild(textarea);
  textarea.select();
  try {
    return doc.execCommand("copy");
  } catch {
    return false;
  } finally {
    textarea.remove();
  }
}
