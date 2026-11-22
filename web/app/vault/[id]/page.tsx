import type { Metadata } from "next";
import { buildVaultJsonLd, buildVaultMetadata, fetchVaultData } from "@/lib/vault-metadata";
import { VaultDetailPageClient } from "@/components/vault/VaultDetailPageClient";

interface VaultPageProps {
  // Next.js 15+ App Router: route params are async (Promise-based).
  params: Promise<{ id: string }>;
}

export async function generateMetadata({ params }: VaultPageProps): Promise<Metadata> {
  const resolvedParams = await params;
  const vault = await fetchVaultData(resolvedParams.id);
  return buildVaultMetadata(vault, resolvedParams.id);
}

export default async function VaultPage({ params }: VaultPageProps) {
  const resolvedParams = await params;
  const vault = await fetchVaultData(resolvedParams.id);
  const jsonLd = buildVaultJsonLd(vault, resolvedParams.id);

  return (
    <>
      {/* Plain text child, not dangerouslySetInnerHTML: script tags are a raw-text
          HTML element, so JSON.stringify's output renders as-is without escaping. */}
      <script type="application/ld+json">{JSON.stringify(jsonLd)}</script>
      <VaultDetailPageClient id={resolvedParams.id} />
    </>
  );
}

export async function generateStaticParams() {
  return [];
}
