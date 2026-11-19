import type { Metadata } from 'next';
import { getTranslations } from 'next-intl/server';

export async function generateMetadata(): Promise<Metadata> {
  const t = await getTranslations();
  return {
    title: t('managers.metaTitle'),
    description: t('managers.metaDescription'),
  };
}

export default async function ManagersPage() {
  const t = await getTranslations();

  return (
    <main className="p-6 max-w-4xl mx-auto">
      <h1 className="text-2xl font-bold text-gray-900 dark:text-gray-100">
        {t('managers.title')}
      </h1>
      <p className="mt-2 text-gray-600 dark:text-gray-400">
        {t('managers.subtitle')}
      </p>
      {/* TODO: wire up manager list from /api/v1/managers */}
      <div className="mt-6 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
        <p className="text-sm text-gray-500 dark:text-gray-400">
          {t('managers.empty')}
        </p>
      </div>
    </main>
  );
}
