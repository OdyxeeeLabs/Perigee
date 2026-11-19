"use client";

import { useTranslations } from "next-intl";
import {
  deriveOnboardingChecklist,
  type OnboardingStatus,
} from "../../lib/onboardingChecklist";

export interface OnboardingChecklistProps {
  /** Status returned by the manager status API, not local form state. */
  status: OnboardingStatus;
  /** Whether a wallet address is currently available. */
  walletConnected: boolean;
  /** API error message, shown separately so an error is never read as approved. */
  error?: string | null;
}

/**
 * API-backed onboarding progress (WEB-20 / #821).
 *
 * Every step carries a literal actor and state label, so status is not
 * communicated by colour or icon alone. Administrator review is a separate
 * step and is never inferred from a completed registration form.
 */
export default function OnboardingChecklist({
  status,
  walletConnected,
  error,
}: OnboardingChecklistProps) {
  const t = useTranslations();
  const steps = deriveOnboardingChecklist(status, walletConnected);

  return (
    <section
      aria-labelledby="onboarding-checklist-heading"
      className="mt-8 rounded-xl border border-slate-800 bg-slate-900/50 p-5"
      data-testid="onboarding-checklist"
    >
      <h3
        id="onboarding-checklist-heading"
        className="mb-4 text-lg font-semibold text-cyan-300"
      >
        {t("onboarding.checklistTitle")}
      </h3>

      <ol className="space-y-3">
        {steps.map((step) => {
          const actorLabel =
            step.actor === "admin"
              ? t("onboarding.checklistActorAdmin")
              : t("onboarding.checklistActorYou");
          const stateLabel =
            step.state === "complete"
              ? t("onboarding.checklistStateComplete")
              : step.state === "blocked"
                ? t("onboarding.checklistStateBlocked")
                : t("onboarding.checklistStatePending");
          const marker = step.state === "complete" ? "✓" : step.state === "blocked" ? "!" : "○";

          return (
            <li
              key={step.key}
              className="flex flex-wrap items-baseline gap-x-3 gap-y-1 text-sm text-slate-300"
              data-testid={`onboarding-step-${step.key}`}
            >
              <span aria-hidden="true" className="w-5 text-cyan-400">
                {marker}
              </span>
              <span className="font-medium">{t(step.labelKey)}</span>
              <span className="text-slate-400">({actorLabel})</span>
              <span className="text-slate-400">{stateLabel}</span>
            </li>
          );
        })}
      </ol>

      {error && (
        <p className="mt-4 border-t border-slate-800 pt-3 text-sm text-red-400" role="alert">
          {t("onboarding.checklistError")}
        </p>
      )}
    </section>
  );
}
