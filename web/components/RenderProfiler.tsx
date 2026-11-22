"use client";

import { Profiler, type ProfilerOnRenderCallback, type ReactNode } from "react";
import { trackTelemetryEvent } from "../lib/telemetry";

/** Interaction render budget from issue #692: one 60fps frame. */
export const RENDER_BUDGET_MS = 16;

/**
 * Wrap a subtree to flag renders that exceed {@link RENDER_BUDGET_MS}.
 * Slow renders are logged in development and sent (consent-gated, anonymous)
 * as a `slow_render` telemetry event in production. The React DevTools
 * Profiler remains the tool for finding the cause.
 */
export function RenderProfiler({ id, children }: { id: string; children: ReactNode }) {
  const onRender: ProfilerOnRenderCallback = (profilerId, phase, actualDuration) => {
    if (actualDuration <= RENDER_BUDGET_MS) return;
    if (process.env.NODE_ENV !== "production") {
      console.warn(
        `[perf] ${profilerId} ${phase} render took ${actualDuration.toFixed(1)}ms (budget ${RENDER_BUDGET_MS}ms)`,
      );
    }
    trackTelemetryEvent({
      name: "slow_render",
      properties: { id: profilerId, phase, duration_ms: Math.round(actualDuration) },
    });
  };

  return (
    <Profiler id={id} onRender={onRender}>
      {children}
    </Profiler>
  );
}
