import type { Metadata } from "next";
import { buildVaultJsonLd, buildVaultMetadata, fetchVaultData } from "@/lib/vault-metadata";
import { VaultDetailPageClient } from "@/components/vault/VaultDetailPageClient";

interface VaultsDetailPageProps {
  // Next.js 15+ App Router: route params are async (Promise-based).
  params: Promise<{ id: string }>;
}

export async function generateMetadata({ params }: VaultsDetailPageProps): Promise<Metadata> {
  const resolvedParams = await params;
  const vault = await fetchVaultData(resolvedParams.id);
  return buildVaultMetadata(vault, resolvedParams.id);
}

export default async function VaultsDetailPage({ params }: VaultsDetailPageProps) {
  const resolvedParams = await params;
  const vault = await fetchVaultData(resolvedParams.id);
  const jsonLd = buildVaultJsonLd(vault, resolvedParams.id);

  return (
    <>
      <script type="application/ld+json">{JSON.stringify(jsonLd)}</script>
      <VaultDetailPageClient id={resolvedParams.id} />
    </>
  );
}

export async function generateStaticParams() {
  return [];
}
