/**
 * Consistent language for destructive confirmations (issue #845 / WEB-44).
 *
 * Every destructive action names its target and its consequence in the same
 * sentence, so users do not have to infer what a bare "Are you sure?" refers
 * to. Kept DOM-free so the wording can be tested without a React renderer.
 */

export interface DestructiveAction {
  /** The action as a short imperative, e.g. "Revoke access for". */
  action: string;
  /** The thing being acted on, e.g. a manager name or a configuration label. */
  target: string;
  /** What happens when the action completes. */
  consequence: string;
}

export function confirmationTitle(action: string): string {
  return `${action.trim().replace(/[?.!]+$/, "")}?`;
}

export function confirmationDescription({
  action,
  target,
  consequence,
}: DestructiveAction): string {
  const verb = action.trim().replace(/[?.!]+$/, "");
  return `${verb} ${target}? ${consequence.trim()} This cannot be undone.`;
}

export function confirmationAnnouncement(action: DestructiveAction): string {
  return `Confirmation required. ${confirmationDescription(action)}`;
}
