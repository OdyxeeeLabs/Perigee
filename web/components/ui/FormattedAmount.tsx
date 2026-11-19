import {
  formatAmount,
  formatPercent,
  formatRawNumber,
  formatSignedPercent,
} from "@/lib/locale/format";
import type { Locale } from "@/lib/locale/locales";

export interface FormattedAmountProps {
  value: number;
  locale: Locale | string;
  asset?: string;
  maximumFractionDigits?: number;
  className?: string;
}

/**
 * A rounded balance whose full-precision value is exposed to assistive
 * technology. The visual text is hidden from the accessibility tree so a
 * screen reader announces only the raw value, never both.
 */
export function FormattedAmount({
  value,
  locale,
  asset,
  maximumFractionDigits = 2,
  className,
}: FormattedAmountProps) {
  const display = formatAmount(value, locale, { maximumFractionDigits });
  const raw = asset
    ? `${formatRawNumber(value, locale)} ${asset}`
    : formatRawNumber(value, locale);

  return (
    <span className={className}>
      <span className="sr-only">{raw}</span>
      <span aria-hidden="true">{display}{asset ? ` ${asset}` : ""}</span>
    </span>
  );
}

export interface FormattedPercentProps {
  /** Percentage points: pass 12.3 for 12.3%. */
  value: number;
  locale: Locale | string;
  digits?: number;
  signed?: boolean;
  className?: string;
}

export function FormattedPercent({
  value,
  locale,
  digits = 2,
  signed = false,
  className,
}: FormattedPercentProps) {
  const display = signed
    ? formatSignedPercent(value, locale, digits)
    : formatPercent(value, locale, digits);
  const raw = `${formatRawNumber(value, locale)}%`;

  return (
    <span className={className}>
      <span className="sr-only">{raw}</span>
      <span aria-hidden="true">{display}</span>
    </span>
  );
}
