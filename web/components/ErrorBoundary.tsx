import React from 'react';
import { reportError, type ErrorBoundaryVariant, type ReportingContext } from '../lib/error-reporting';
import { logger } from '../lib/logger';
import { trackTelemetryError } from '../lib/telemetry';

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

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error, errorInfo: null };
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
    this.setState({ errorInfo });
    reportError(error, resolveContext(this.props));
    logger.error('Unhandled UI error:', error, errorInfo);
    trackTelemetryError(error, { component: 'ErrorBoundary' });
  }

  componentDidUpdate(prevProps: ErrorBoundaryProps) {
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

      return (
        <div className="flex min-h-[260px] items-center justify-center rounded-lg border border-red-900/60 bg-[#0d1117] p-6 text-red-100">
          <div className="w-full max-w-xl rounded-lg border border-red-800/60 bg-red-950/30 p-6 shadow-xl shadow-black/20">
            <div className="flex items-start gap-4">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full border border-red-700/70 bg-red-950 text-lg font-bold">
                !
              </div>
              <div className="min-w-0 flex-1">
                <h2 className="text-base font-semibold text-red-50">{title}</h2>
                <p className="mt-2 text-sm leading-6 text-red-100/80">{description}</p>
                <p className="mt-3 rounded-md border border-red-900/70 bg-black/20 px-3 py-2 font-mono text-xs text-red-100/80">
                  {this.state.error.message || this.state.error.name}
                </p>
                {process.env.NODE_ENV !== 'production' && this.state.errorInfo?.componentStack && (
                  <details className="mt-3 text-xs text-red-100/70">
                    <summary className="cursor-pointer text-red-100">Component stack</summary>
                    <pre className="mt-2 max-h-40 overflow-auto whitespace-pre-wrap rounded-md bg-black/30 p-3">
                      {this.state.errorInfo.componentStack}
                    </pre>
                  </details>
                )}
                <div className="mt-5 flex flex-wrap gap-3">
                  <button
                    type="button"
                    onClick={this.reset}
                    className="rounded-md border border-red-700/70 px-4 py-2 text-sm font-medium text-red-50 transition-colors hover:bg-red-900/40"
                  >
                    Try again
                  </button>
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
