"use client";

import { useId } from "react";
import { useFocusTrap } from "@/hooks/useFocusTrap";
import { LiveStatus } from "@/components/LiveRegion";
import {
  confirmationAnnouncement,
  confirmationDescription,
  confirmationTitle,
  type DestructiveAction,
} from "@/lib/a11y/confirmation";
import { Button } from "./Button";

export interface ConfirmDialogProps extends DestructiveAction {
  open: boolean;
  onConfirm: () => void;
  onCancel: () => void;
  confirmLabel?: string;
  cancelLabel?: string;
}

/**
 * Shared confirmation for irreversible actions (issue #845 / WEB-44).
 *
 * Focus lands on the dialog rather than on the destructive button, Escape and
 * Cancel are equivalent, and the action runs only from the explicit confirm
 * handler. Callers never call their API before `onConfirm` fires.
 */
export function ConfirmDialog({
  open,
  action,
  target,
  consequence,
  onConfirm,
  onCancel,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
}: ConfirmDialogProps) {
  const id = useId();
  const titleId = `${id}-title`;
  const descriptionId = `${id}-description`;
  const { ref } = useFocusTrap<HTMLDivElement>({
    active: open,
    onEscape: onCancel,
    initialFocus: "container",
  });

  if (!open) return null;

  const description = confirmationDescription({ action, target, consequence });

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center p-4">
      <div
        className="absolute inset-0 bg-black/60 backdrop-blur-sm"
        onClick={onCancel}
        aria-hidden="true"
      />
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={descriptionId}
        data-testid="confirm-dialog"
        className="relative z-10 w-full max-w-md rounded-xl border border-slate-700 bg-slate-900 p-6 shadow-2xl focus:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400"
      >
        <h2 id={titleId} className="text-lg font-semibold text-white">
          {confirmationTitle(action)}
        </h2>
        <p id={descriptionId} className="mt-2 text-sm text-slate-300">
          {description}
        </p>
        <div className="mt-6 flex justify-end gap-2">
          <Button type="button" variant="outline" onClick={onCancel} data-testid="confirm-cancel">
            {cancelLabel}
          </Button>
          <Button
            type="button"
            variant="destructive"
            onClick={onConfirm}
            data-testid="confirm-action"
          >
            {confirmLabel}
          </Button>
        </div>
        <LiveStatus
          message={confirmationAnnouncement({ action, target, consequence })}
          data-testid="confirm-live-status"
        />
      </div>
    </div>
  );
}

export default ConfirmDialog;
