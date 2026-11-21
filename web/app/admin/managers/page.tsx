import type { Metadata } from 'next';
import { getTranslations } from 'next-intl/server';
import AdminManagersClient from '../../../components/admin/AdminManagersClient';

export async function generateMetadata(): Promise<Metadata> {
  const t = await getTranslations();
  return {
    title: t('admin.managers.directoryMetaTitle'),
    description: t('admin.managers.directoryMetaDescription'),
  };
}

export default async function AdminManagersPage() {
  const t = await getTranslations();

  return (
    <main className="p-6 max-w-4xl mx-auto">
      <h1 className="text-2xl font-bold text-gray-900 dark:text-gray-100">
        {t('admin.managers.directoryTitle')}
      </h1>
      <p className="mt-2 text-gray-600 dark:text-gray-400">
        {t('admin.managers.directorySubtitle')}
      </p>
      {/* TODO: wire up manager list from /api/v1/admin/managers */}
      <div className="mt-6 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
        <p className="text-sm text-gray-500 dark:text-gray-400">
          {t('admin.managers.directoryEmpty')}
        </p>
      </div>
      <AdminManagersClient />
    </main>
  );
}
