import Head from "next/head";
import { useTranslations } from "next-intl";

/**
 * SEO — Centralized per-page metadata.
 *
 * Wraps `next/head` so every Pages-Router page renders, at minimum:
 *   • `title`
 *   • `description`
 *   • canonical URL
 *   • Open Graph + Twitter Card tags for sharing and indexing
 *
 * Pages must use `<SEO title="…" description="…" path="/…" />` instead of
 * raw `<Head>` blocks. Closes #104 / WEB-18.
 *
 * Defaults:
 *   - `path` is appended to the canonical site origin (https://perigee.app).
 *   - `ogImage` falls back to `/og-default.png`.
 */
export interface SeoProps {
  /** Page-specific title; " | Perigee" is appended automatically. */
  title: string;
  /** One-sentence summary that explains what the page is about. */
  description: string;
  /** Path component of the canonical URL, e.g. "/admin/managers". */
  path?: string;
  /** Absolute or root-relative image URL for shared previews. */
  ogImage?: string;
  /** Open Graph type, e.g. "website", "article", or custom type. Defaults to "website". */
  ogType?: string;
  /** When `true`, adds `<meta name="robots" content="noindex,nofollow">`. */
  noIndex?: boolean;
  /** Schema.org structured data (JSON-LD) object or array of objects. */
  jsonLd?: Record<string, unknown> | Array<Record<string, unknown>>;
  /** Additional custom Open Graph / meta properties (e.g. vault:name, vault:balance, vault:apy). */
  customOg?: Record<string, string | number | undefined | null>;
  /** Twitter card type. Defaults to "summary_large_image". */
  twitterCard?: "summary" | "summary_large_image" | "app" | "player";
}

// Allow staging / preview deployments to advertise their own origin so
// canonical URLs and OG tags don't lie about the host a page was rendered at.
export const SITE_ORIGIN =
  process.env.NEXT_PUBLIC_SITE_URL?.replace(/\/+$/, "") || "https://perigee.app";
export const DEFAULT_OG_IMAGE = "/og-default.png";

export function buildTitle(title: string, siteName: string = "Perigee"): string {
  if (new RegExp(siteName, "i").test(title) || /(Perigee)/i.test(title)) {
    return title;
  }
  return `${title} | ${siteName}`;
}

export function buildCanonical(path?: string): string | undefined {
  if (!path) return undefined;
  return `${SITE_ORIGIN}${path.startsWith("/") ? path : `/${path}`}`;
}

export function SEO({
  title,
  description,
  path,
  ogImage,
  ogType = "website",
  noIndex = false,
  jsonLd,
  customOg,
  twitterCard = "summary_large_image",
}: SeoProps) {
  let siteName = "Perigee";
  let defaultTitle = "Perigee - Soroban Smart Contract Resource Analyzer";
  let defaultDescription =
    "Explore, test, and analyze the CPU, RAM, and ledger footprint of Soroban smart contracts.";

  try {
    const t = useTranslations();
    siteName = t("seo.siteName") || siteName;
    defaultTitle = t("seo.defaultTitle") || defaultTitle;
    defaultDescription = t("seo.defaultDescription") || defaultDescription;
  } catch {
    // Fallback if not wrapped in NextIntlProvider
  }

  const effectiveTitle = title || defaultTitle;
  const effectiveDescription = description || defaultDescription;
  const fullTitle = buildTitle(effectiveTitle, siteName);
  const canonical = buildCanonical(path);
  const image = ogImage ?? DEFAULT_OG_IMAGE;

  return (
    <Head>
      <title>{fullTitle}</title>
      <meta name="description" content={effectiveDescription} />
      {noIndex && <meta name="robots" content="noindex,nofollow" />}
      {canonical && <link rel="canonical" href={canonical} />}

      {/* Open Graph */}
      <meta property="og:title" content={fullTitle} />
      <meta property="og:description" content={description} />
      <meta property="og:type" content={ogType} />
      <meta property="og:image" content={image} />
      {canonical && <meta property="og:url" content={canonical} />}
      <meta property="og:site_name" content={siteName} />

      {/* Custom Open Graph tags (e.g. vault metrics) */}
      {customOg &&
        Object.entries(customOg).map(([key, val]) => {
          if (val === undefined || val === null) return null;
          const property =
            key.startsWith("og:") || key.startsWith("vault:") || key.includes(":")
              ? key
              : `og:${key}`;
          return <meta key={property} property={property} content={String(val)} />;
        })}

      {/* Twitter / X */}
      <meta name="twitter:card" content={twitterCard} />
      <meta name="twitter:title" content={fullTitle} />
      <meta name="twitter:description" content={description} />
      <meta name="twitter:image" content={image} />

      {/* Structured Data (JSON-LD) */}
      {jsonLd && (
        <script
          type="application/ld+json"
          dangerouslySetInnerHTML={{ __html: JSON.stringify(jsonLd) }}
        />
      )}
    </Head>
  );
}
