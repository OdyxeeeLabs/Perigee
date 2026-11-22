import { useEffect, useState } from 'react';

type Theme = 'light' | 'dark' | 'system';

export function ThemeToggle() {
  const [theme, setTheme] = useState<Theme>('system');

  useEffect(() => {
    const stored = localStorage.getItem('theme') as Theme | null;
    if (stored) setTheme(stored);
  }, []);

  useEffect(() => {
    const root = document.documentElement;
    if (theme === 'system') {
      const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
      root.classList.toggle('dark', prefersDark);
      localStorage.removeItem('theme');
    } else {
      root.classList.toggle('dark', theme === 'dark');
      localStorage.setItem('theme', theme);
    }
  }, [theme]);

  // Also listen for system preference changes
  useEffect(() => {
    if (theme !== 'system') return;
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const handler = (e: MediaQueryListEvent) => {
      document.documentElement.classList.toggle('dark', e.matches);
    };
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  }, [theme]);

  return (
    <div className="flex items-center gap-2" aria-label="Theme selector">
      <button
        onClick={() => setTheme('light')}
        className={`px-3 py-1 rounded text-sm font-medium transition-colors ${
          theme === 'light'
            ? 'bg-gray-200 dark:bg-gray-700 text-gray-900 dark:text-gray-100'
            : 'text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-gray-100'
        }`}
        aria-pressed={theme === 'light'}
      >
        Light
      </button>
      <button
        onClick={() => setTheme('system')}
        className={`px-3 py-1 rounded text-sm font-medium transition-colors ${
          theme === 'system'
            ? 'bg-gray-200 dark:bg-gray-700 text-gray-900 dark:text-gray-100'
            : 'text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-gray-100'
        }`}
        aria-pressed={theme === 'system'}
      >
        System
      </button>
      <button
        onClick={() => setTheme('dark')}
        className={`px-3 py-1 rounded text-sm font-medium transition-colors ${
          theme === 'dark'
            ? 'bg-gray-200 dark:bg-gray-700 text-gray-900 dark:text-gray-100'
            : 'text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-gray-100'
        }`}
        aria-pressed={theme === 'dark'}
      >
        Dark
      </button>
    </div>
  );
}
