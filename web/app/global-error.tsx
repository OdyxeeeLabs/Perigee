'use client';

import { useEffect, useRef } from 'react';
import { reportError } from '../lib/error-reporting';
import { focusRecovery } from '../lib/a11y/focus';

export default function GlobalError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  const recoveryRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    reportError(error, {
      boundary: 'root',
      route: typeof window !== 'undefined' ? window.location.pathname : '/',
      component: 'GlobalError',
    });
  }, [error]);

  useEffect(() => {
    focusRecovery(recoveryRef.current);
  }, []);

  return (
    <html lang="en">
      <body className="bg-slate-950 text-slate-50">
        <div className="flex min-h-screen items-center justify-center bg-slate-950 px-4 text-slate-50">
          <div
            ref={recoveryRef}
            role="alert"
            aria-labelledby="global-error-title"
            tabIndex={-1}
            className="w-full max-w-lg rounded-xl border border-red-800/70 bg-slate-900 p-6 shadow-2xl shadow-black/30 focus:outline-none focus:ring-2 focus:ring-red-400"
          >
            <h1 id="global-error-title" className="text-xl font-semibold text-red-100">
              Something went wrong
            </h1>
            <p className="mt-3 text-sm leading-6 text-slate-300">
              The application hit an unexpected error. You can retry the session or reload the page.
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
                Reload app
              </button>
            </div>
          </div>
        </div>
      </body>
    </html>
  );
}
