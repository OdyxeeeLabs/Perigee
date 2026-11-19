/**
 * Pure state mapping for the manager onboarding checklist (WEB-20 / #821).
 *
 * The checklist is derived only from API-backed registration status and the
 * wallet connection flag. Local form state never marks a step complete, and an
 * administrator review is never inferred from form completion.
 */

export type OnboardingStatus = "unregistered" | "pending" | "approved" | "rejected";

export type ChecklistActor = "user" | "admin";
export type ChecklistState = "complete" | "pending" | "blocked";

export type ChecklistLabelKey =
  | "onboarding.checklistStepWallet"
  | "onboarding.checklistStepDetails"
  | "onboarding.checklistStepKyc"
  | "onboarding.checklistStepReview";

export interface ChecklistStep {
  key: "wallet" | "details" | "kyc" | "review";
  labelKey: ChecklistLabelKey;
  actor: ChecklistActor;
  state: ChecklistState;
}

/**
 * Build the ordered checklist for a manager onboarding lifecycle state.
 *
 * - `wallet` is complete only when a wallet is actually connected.
 * - `details` and `kyc` are complete only once a registration exists (any
 *   status other than `unregistered`), because only the API knows that the
 *   submission was accepted.
 * - `review` belongs to the administrator. It is complete only for `approved`,
 *   blocked for `rejected`, and pending otherwise.
 */
export function deriveOnboardingChecklist(
  status: OnboardingStatus,
  walletConnected: boolean,
): ChecklistStep[] {
  const submitted = status !== "unregistered";

  const reviewState: ChecklistState =
    status === "approved" ? "complete" : status === "rejected" ? "blocked" : "pending";

  return [
    {
      key: "wallet",
      labelKey: "onboarding.checklistStepWallet",
      actor: "user",
      state: walletConnected ? "complete" : "pending",
    },
    {
      key: "details",
      labelKey: "onboarding.checklistStepDetails",
      actor: "user",
      state: submitted ? "complete" : "pending",
    },
    {
      key: "kyc",
      labelKey: "onboarding.checklistStepKyc",
      actor: "user",
      state: submitted ? "complete" : "pending",
    },
    {
      key: "review",
      labelKey: "onboarding.checklistStepReview",
      actor: "admin",
      state: reviewState,
    },
  ];
}
