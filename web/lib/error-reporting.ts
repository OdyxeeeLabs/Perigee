import * as Sentry from '@sentry/nextjs';
import { logger, sanitizeForLog } from './logger';

export type ErrorBoundaryVariant = 'root' | 'route' | 'component';

export type ReportingContext = {
  boundary?: ErrorBoundaryVariant;
  route?: string;
  component?: string;
  user?: {
    id?: string;
    email?: string;
    username?: string;
    [key: string]: string | undefined;
  };
  props?: Record<string, unknown>;
};

const getCurrentRoute = (): string => {
  if (typeof window === 'undefined') {
    return '/';
  }

  return window.location.pathname || '/';
};

export function reportError(error: unknown, context: ReportingContext = {}): void {
  const normalizedError = error instanceof Error ? error : new Error(String(error));
  const payload = {
    message: normalizedError.message,
    name: normalizedError.name,
    stack: normalizedError.stack,
    ...sanitizeForLog(context),
    route: context.route ?? getCurrentRoute(),
  };

  logger.error('[ErrorBoundary]', payload);

  const dsn = process.env.NEXT_PUBLIC_SENTRY_DSN ?? process.env.SENTRY_DSN;
  if (!dsn) {
    return;
  }

  try {
    Sentry.withScope((scope) => {
      if (context.boundary) {
        scope.setTag('boundary', context.boundary);
      }

      if (context.component) {
        scope.setTag('component', context.component);
      }

      if (context.route) {
        scope.setTag('route', context.route);
      }

      if (context.user) {
        scope.setUser(context.user);
      }

      if (context.props) {
        scope.setContext('props', sanitizeForLog(context.props) as Record<string, unknown>);
      }

      scope.setContext('route_context', {
        route: context.route ?? getCurrentRoute(),
      });

      Sentry.captureException(normalizedError);
    });
  } catch (reportingError) {
    logger.error('[ErrorReporting]', sanitizeForLog(reportingError));
  }
}
