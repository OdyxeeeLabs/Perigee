import React from 'react';

type ErrorBoundaryProps = {
  children: React.ReactNode;
  fallback?: (error: Error, reset: () => void) => React.ReactNode;
  title?: string;
  description?: string;
};

type ErrorBoundaryState = {
  error: Error | null;
  errorInfo: React.ErrorInfo | null;
};

export class ErrorBoundary extends React.Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null, errorInfo: null };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error, errorInfo: null };
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
    this.setState({ errorInfo });
    console.error('Unhandled UI error:', error, errorInfo);
  }

  reset = () => {
    this.setState({ error: null, errorInfo: null });
  };

  render() {
    if (this.state.error) {
      if (this.props.fallback) {
        return this.props.fallback(this.state.error, this.reset);
      }

      return (
        <div className="flex min-h-[320px] items-center justify-center rounded-lg border border-red-900/60 bg-[#0d1117] p-6 text-red-100">
          <div className="w-full max-w-xl rounded-lg border border-red-800/60 bg-red-950/30 p-6 shadow-xl shadow-black/20">
            <div className="flex items-start gap-4">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full border border-red-700/70 bg-red-950 text-lg font-bold">
                !
              </div>
              <div className="min-w-0 flex-1">
                <h2 className="text-base font-semibold text-red-50">
                  {this.props.title ?? 'Something went wrong'}
                </h2>
                <p className="mt-2 text-sm leading-6 text-red-100/80">
                  {this.props.description ??
                    'A dashboard component crashed while rendering. You can retry without losing the rest of the app.'}
                </p>
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
                    onClick={() => window.location.reload()}
                    className="rounded-md border border-slate-700 px-4 py-2 text-sm font-medium text-slate-100 transition-colors hover:bg-slate-800"
                  >
                    Reload dashboard
                  </button>
                </div>
              </div>
            </div>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}
