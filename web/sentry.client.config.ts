import * as Sentry from '@sentry/nextjs';

if (process.env.NEXT_PUBLIC_SENTRY_DSN) {
  Sentry.init({
    dsn: process.env.NEXT_PUBLIC_SENTRY_DSN,
    enabled: process.env.NODE_ENV !== 'test',
    environment: process.env.NODE_ENV,
    tracesSampleRate: 1.0,
    sendDefaultPii: false,
  });
}
