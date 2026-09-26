import React from "react";
import styles from "./Button.module.css";

export interface CommonButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary" | "outline";
  loading?: boolean;
}

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