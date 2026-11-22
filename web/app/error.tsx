'use client';

import { useEffect, useRef } from 'react';
import { reportError } from '../lib/error-reporting';
import { focusRecovery } from '../lib/a11y/focus';

export default function AppRouteError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  const recoveryRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    reportError(error, {
      boundary: 'route',
      route: typeof window !== 'undefined' ? window.location.pathname : '/',
      component: 'AppRouteError',
    });
  }, [error]);

  useEffect(() => {
    focusRecovery(recoveryRef.current);
  }, []);

  return (
    <div className="flex min-h-screen items-center justify-center bg-slate-950 px-4 text-slate-50">
      <div
        ref={recoveryRef}
        role="alert"
        aria-labelledby="route-error-title"
        tabIndex={-1}
        className="w-full max-w-lg rounded-xl border border-red-800/70 bg-slate-900 p-6 shadow-2xl shadow-black/30 focus:outline-none focus:ring-2 focus:ring-red-400"
      >
        <h1 id="route-error-title" className="text-xl font-semibold text-red-100">
          This page failed to load
        </h1>
        <p className="mt-3 text-sm leading-6 text-slate-300">
          The route hit an unexpected error during render. Try again to recover without reloading the app.
        </p>
        <div className="mt-5 flex flex-wrap gap-3">
          <button
            type="button"
            onClick={reset}
            className="rounded-md border border-red-700 bg-red-900/40 px-4 py-2 text-sm font-medium text-red-50 hover:bg-red-900/60"
          >
            Try again
          </button>
          <a
            href="/"
            className="rounded-md border border-slate-700 px-4 py-2 text-sm font-medium text-slate-100 hover:bg-slate-800"
          >
            Back to dashboard
          </a>
          <button
            type="button"
            onClick={() => {
              if (typeof window !== 'undefined') {
                window.location.reload();
              }
            }}
            className="rounded-md border border-slate-700 px-4 py-2 text-sm font-medium text-slate-100 hover:bg-slate-800"
          >
            Reload page
          </button>
        </div>
      </div>
    </div>
  );
}
