import React from "react";

export interface FormSkeletonProps {
  /** Number of label+input field placeholders to render. Defaults to 4. */
  fields?: number;
  /** Accessible label for the loading region. */
  label?: string;
  /** Extra Tailwind classes merged onto the root. */
  className?: string;
}

/**
 * Generic content-aware skeleton for forms — issue #685.
 *
 * Matches the GitHub-dark skeleton convention established by
 * `NutritionLabelSkeleton` / `ResultViewerSkeleton` (`animate-pulse`,
 * `#161b22`/`#30363d`/`#c9d1d9` palette, `role`/`aria-busy` wiring) so any
 * form that loads async config (e.g. the Settings page hydrating from
 * `localStorage`, or a dynamic contract-function form) can show a
 * dimensionally-accurate placeholder instead of a spinner or blank panel.
 */
export const FormSkeleton: React.FC<FormSkeletonProps> = ({
  fields = 4,
  label = "Form loading",
  className = "",
}) => {
  return (
    <div
      className={`bg-[#161b22] border border-[#30363d] rounded-lg p-6 font-mono animate-pulse ${className}`}
      role="group"
      aria-label={label}
      aria-busy="true"
    >
      <div className="flex flex-col gap-5">
        {Array.from({ length: fields }).map((_, idx) => (
          <div key={idx} className="flex flex-col gap-2">
            <div className="h-3 w-24 bg-[#30363d] rounded" />
            <div className="h-9 w-full max-w-md bg-[#0d1117] border border-[#30363d] rounded" />
          </div>
        ))}
      </div>

      <div className="mt-6 flex gap-3">
        <div className="h-9 w-28 bg-[#30363d] rounded" />
        <div className="h-9 w-20 bg-[#0d1117] border border-[#30363d] rounded" />
      </div>
    </div>
  );
};
