import React from 'react';
import { reportError, type ErrorBoundaryVariant, type ReportingContext } from '../lib/error-reporting';
import { focusRecovery } from '../lib/a11y/focus';
import { logger } from '../lib/logger';
import { getTelemetryConsent, trackTelemetryError } from '../lib/telemetry';

type ErrorBoundaryProps = {
  children?: React.ReactNode;
  fallback?: (
    error: Error,
    reset: () => void,
    context: ReportingContext,
  ) => React.ReactNode;
  title?: string;
  description?: string;
  context?: Partial<ReportingContext>;
  variant?: ErrorBoundaryVariant;
  resetKeys?: unknown[];
  onReset?: () => void;
};

type ErrorBoundaryState = {
  error: Error | null;
  errorInfo: React.ErrorInfo | null;
};

let boundarySequence = 0;

const getCurrentRoute = (): string => {
  if (typeof window === 'undefined') {
    return '/';
  }

  return window.location.pathname || '/';
};

const resolveContext = (props: ErrorBoundaryProps): ReportingContext => ({
  boundary: props.variant ?? 'component',
  route: props.context?.route ?? getCurrentRoute(),
  component: props.context?.component ?? 'UnknownComponent',
  props: props.context?.props,
  user: props.context?.user,
});

export class ErrorBoundary extends React.Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null, errorInfo: null };

  private readonly headingId = `error-boundary-title-${++boundarySequence}`;
  private recoveryRef = React.createRef<HTMLDivElement>();

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error, errorInfo: null };
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
    this.setState({ errorInfo });
    reportError(error, resolveContext(this.props));
    logger.error('Unhandled UI error:', error, errorInfo);

    if (getTelemetryConsent() === true) {
      trackTelemetryError(error, {
        component: resolveContext(this.props).component,
        path: getCurrentRoute(),
      });
    }
  }

  componentDidUpdate(prevProps: ErrorBoundaryProps, prevState: ErrorBoundaryState) {
    if (!prevState.error && this.state.error) {
      focusRecovery(this.recoveryRef.current);
    }
    if (this.state.error && prevProps.resetKeys !== this.props.resetKeys) {
      this.reset();
    }
  }

  reset = () => {
    this.setState({ error: null, errorInfo: null });
    this.props.onReset?.();
  };

  render() {
    if (this.state.error) {
      const context = resolveContext(this.props);

      if (this.props.fallback) {
        return this.props.fallback(this.state.error, this.reset, context);
      }

      const title =
        this.props.title ??
        (this.props.variant === 'root'
          ? 'Something went wrong'
          : this.props.variant === 'route'
            ? 'This page failed to load'
            : 'This widget failed to load');

      const description =
        this.props.description ??
        (this.props.variant === 'root'
          ? 'The app hit an unexpected issue. You can retry without losing the rest of your session.'
          : this.props.variant === 'route'
            ? 'This page hit an unexpected rendering error. Try again or reload the page to recover.'
            : 'This section could not render correctly, but the rest of the dashboard is still available.');

      const isRootOrRoute = this.props.variant === 'root' || this.props.variant === 'route';

      return (
        <div className="flex min-h-[260px] items-center justify-center rounded-lg border border-red-900/60 bg-[#0d1117] p-6 text-red-100">
          <div
            ref={this.recoveryRef}
            role="alert"
            aria-labelledby={this.headingId}
            tabIndex={-1}
            className="w-full max-w-xl rounded-lg border border-red-800/60 bg-red-950/30 p-6 shadow-xl shadow-black/20 focus:outline-none focus:ring-2 focus:ring-red-400"
          >
            <div className="flex items-start gap-4">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full border border-red-700/70 bg-red-950 text-lg font-bold" aria-hidden="true">
                !
              </div>
              <div className="min-w-0 flex-1">
                <h2 id={this.headingId} className="text-base font-semibold text-red-50">{title}</h2>
                <p className="mt-2 text-sm leading-6 text-red-100/80">{description}</p>
                <div className="mt-5 flex flex-wrap gap-3">
                  <button
                    type="button"
                    onClick={this.reset}
                    className="rounded-md border border-red-700/70 px-4 py-2 text-sm font-medium text-red-50 transition-colors hover:bg-red-900/40"
                  >
                    Try again
                  </button>
                  {this.props.variant === 'route' && (
                    <a
                      href="/"
                      className="rounded-md border border-slate-700 px-4 py-2 text-sm font-medium text-slate-100 transition-colors hover:bg-slate-800"
                    >
                      Back to dashboard
                    </a>
                  )}
                  {isRootOrRoute && (
                    <button
                      type="button"
                      onClick={() => {
                        if (typeof window !== 'undefined') {
                          window.location.reload();
                        }
                      }}
                      className="rounded-md border border-slate-700 px-4 py-2 text-sm font-medium text-slate-100 transition-colors hover:bg-slate-800"
                    >
                      Reload page
                    </button>
                  )}
                </div>
              </div>
            </div>
          </div>
        </div>
      );
    }

    return this.props.children ?? null;
  }
}

export const RootErrorBoundary = (props: Omit<ErrorBoundaryProps, 'variant'>) => (
  <ErrorBoundary variant="root" {...props} />
);

export const RouteErrorBoundary = (props: Omit<ErrorBoundaryProps, 'variant'>) => (
  <ErrorBoundary variant="route" {...props} />
);

export const ComponentErrorBoundary = (props: Omit<ErrorBoundaryProps, 'variant'>) => (
  <ErrorBoundary variant="component" {...props} />
);
