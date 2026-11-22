import React from "react";
import styles from "./Button.module.css";

export interface CommonButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary" | "outline";
  loading?: boolean;
}

/**
 * Legacy button from the pre-`ui/` component set — **prefer
 * [`Button`](./button.md)**, which adds `size`, the full variant set, and
 * native prop forwarding.
 *
 * @deprecated Superseded by `components/ui/Button`. The variant names changed
 * (`primary` → `default`) and `loading` folded into the native `disabled`
 * attribute. See the [migration guide](./migrations.md).
 *
 * @example
 * // Deprecated
 * <CommonButton variant="primary" loading>Save</CommonButton>
 *
 * @example
 * // Replacement
 * <Button disabled={loading}>Save</Button>
 *
 * @accessibility
 * - Adding `loading` also sets `disabled`, so the control is removed from the
 *   tab order while work is in flight and screen readers announce it as
 *   unavailable rather than busy.
 *
 * @since 0.0.1
 */
export const CommonButton: React.FC<CommonButtonProps> = ({
  children,
  variant = "primary",
  loading = false,
  className = "",
  disabled,
  ...props
}) => {
  return (
    <button
      className={`${styles.button} ${styles[variant]} ${
        loading ? styles.loading : ""
      } ${className}`}
      disabled={disabled || loading}
      {...props}
    >
      {children}
    </button>
  );
};

export default CommonButton;