'use client';

import React from "react"

import { useState } from 'react';
import { Loader2 } from 'lucide-react';
import { useTranslations } from 'next-intl';
import { sanitizeUserInput } from '../lib/sanitize';
import type { ContractFunction, SimulationInputs } from '../lib/sorobantypes';

interface DynamicFormProps {
  func: ContractFunction;
  onSubmit: (inputs: SimulationInputs) => void;
  loading?: boolean;
}

// ---------------------------------------------------------------------------
// Validation helpers (also exported for unit testing)
// ---------------------------------------------------------------------------

/**
 * Validate a Stellar address.
 *
 * Stellar uses Strkey encoding (base32):
 *   - Public keys (Ed25519)  start with 'G' and are 56 characters long.
 *   - Contract addresses     start with 'C' and are 56 characters long.
 *
 * Returns an error string on failure, or null when valid.
 */
export function validateStellarAddress(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null; // let `required` handle emptiness

  // Stellar's strkey encoding uses standard RFC 4648 base32: A-Z and digits 2-7.
  const STRKEY_RE = /^[A-Z2-7]{56}$/;

  if (trimmed.startsWith('G') || trimmed.startsWith('C')) {
    if (!STRKEY_RE.test(trimmed)) {
      return `Must be 56 base32 characters starting with ${trimmed[0]}.`;
    }
    return null;
  }

  return "Stellar address must start with 'G' (Ed25519 public key) or 'C' (contract).";
}

/**
 * Validate a Stellar asset code (alphanumeric, 1–12 characters).
 *
 * Returns an error string on failure, or null when valid.
 */
export function validateAssetCode(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null; // let `required` handle emptiness

  if (!/^[A-Za-z0-9]{1,12}$/.test(trimmed)) {
    if (trimmed.length > 12) {
      return 'Asset code must be 1–12 characters.';
    }
    return 'Asset code must contain only letters and numbers (A-Z, 0-9).';
  }
  return null;
}

/**
 * Validate a single field value given its SorobanType.
 * Returns an error string, or null if valid.
 */
export function validateField(type: string, value: string): string | null {
  switch (type) {
    case 'address':
      return validateStellarAddress(value);
    case 'asset_code':
      return validateAssetCode(value);
    default:
      return null;
  }
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function DynamicForm({ func, onSubmit, loading }: DynamicFormProps) {
  const t = useTranslations();
  const [formData, setFormData] = useState<SimulationInputs>({});
  // Map of field name → error message (empty string or undefined = no error)
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});

  const handleChange = (name: string, type: string, value: string | number | boolean) => {
    setFormData((prev) => ({ ...prev, [name]: value }));

    // Validate on change so errors clear as the user types valid input
    const error = validateField(type, String(value));
    setFieldErrors((prev) => ({
      ...prev,
      [name]: error ?? '',
    }));
  };

  const fieldValue = (name: string) => {
    const value = formData[name];
    return typeof value === 'boolean' ? String(value) : value ?? '';
  };

  const hasErrors = Object.values(fieldErrors).some((e) => e !== '');

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();

    // Run validation for every field before submitting.
    // Sanitization is applied before the form payload leaves the browser.
    const sanitizedInputs: SimulationInputs = {};
    const errors: Record<string, string> = {};
    func.inputs.forEach((input) => {
      const raw = String(formData[input.name] ?? '');
      const sanitized = sanitizeUserInput(raw, input.name);
      sanitizedInputs[input.name] = sanitized;
      const error = validateField(input.type, sanitized);
      if (error) errors[input.name] = error;
    });

    if (Object.keys(errors).length > 0) {
      setFieldErrors((prev) => ({ ...prev, ...errors }));
      return;
    }

    onSubmit(sanitizedInputs);
  };

  // Shared input style — border turns red when there is a field error
  const inputStyle = (name: string): React.CSSProperties => ({
    padding: '8px 12px',
    border: `1px solid ${fieldErrors[name] ? '#f85149' : '#30363d'}`,
    borderRadius: '6px',
    fontSize: '14px',
    boxSizing: 'border-box',
    backgroundColor: '#0d1117',
    color: '#c9d1d9',
  });

  return (
    <form onSubmit={handleSubmit} style={{ display: 'flex', flexDirection: 'column', gap: '16px' }}>
      {func.inputs.length === 0 ? (
        <p style={{ color: '#8b949e', fontSize: '14px' }}>{t("dynamicForm.noInputs")}</p>
      ) : (
        func.inputs.map((input) => (
          <div
            key={input.name}
            style={{
              display: 'flex',
              flexDirection: 'column',
              gap: '4px',
            }}
          >
            <label
              htmlFor={`field-${input.name}`}
              style={{
                fontSize: '14px',
                fontWeight: '500',
                color: '#c9d1d9',
              }}
            >
              {input.name}
              {input.optional ? (
                <span style={{ color: '#8b949e', marginLeft: '4px' }}>{t("dynamicForm.optional")}</span>
              ) : (
                <span style={{ color: '#fb8500' }}>*</span>
              )}
            </label>
            {input.description && (
              <p
                style={{
                  fontSize: '12px',
                  color: '#8b949e',
                  margin: '0',
                }}
              >
                {input.description}
              </p>
            )}

            {/* ── Field inputs ── */}
            {input.type === 'address' ? (
              <input
                id={`field-${input.name}`}
                type="text"
                placeholder={t("dynamicForm.placeholderAddress")}
                value={fieldValue(input.name)}
                onChange={(e) => handleChange(input.name, input.type, e.target.value)}
                required={!input.optional}
                disabled={loading}
                aria-invalid={Boolean(fieldErrors[input.name])}
                aria-describedby={fieldErrors[input.name] ? `error-${input.name}` : undefined}
                style={{ ...inputStyle(input.name), fontFamily: 'monospace' }}
              />
            ) : input.type === 'asset_code' ? (
              <input
                id={`field-${input.name}`}
                type="text"
                placeholder={t("dynamicForm.placeholderAssetCode")}
                value={fieldValue(input.name)}
                onChange={(e) => handleChange(input.name, input.type, e.target.value)}
                required={!input.optional}
                disabled={loading}
                aria-invalid={Boolean(fieldErrors[input.name])}
                aria-describedby={fieldErrors[input.name] ? `error-${input.name}` : undefined}
                style={{ ...inputStyle(input.name), fontFamily: 'monospace' }}
              />
            ) : input.type === 'u32' || input.type === 'u128' || input.type === 'i128' ? (
              <input
                id={`field-${input.name}`}
                type="number"
                placeholder={t("dynamicForm.placeholderNumber", { type: input.type })}
                value={fieldValue(input.name)}
                onChange={(e) => handleChange(input.name, input.type, e.target.value)}
                required={!input.optional}
                disabled={loading}
                style={inputStyle(input.name)}
              />
            ) : input.type === 'string' || input.type === 'symbol' ? (
              <input
                id={`field-${input.name}`}
                type="text"
                placeholder={t("dynamicForm.placeholderText", { type: input.type })}
                value={fieldValue(input.name)}
                onChange={(e) => handleChange(input.name, input.type, e.target.value)}
                required={!input.optional}
                disabled={loading}
                style={inputStyle(input.name)}
              />
            ) : input.type === 'bool' ? (
              <select
                id={`field-${input.name}`}
                value={formData[input.name] === undefined ? '' : String(formData[input.name])}
                onChange={(e) => handleChange(input.name, input.type, e.target.value === 'true')}
                required={!input.optional}
                disabled={loading}
                style={inputStyle(input.name)}
              >
                <option value="">{t("dynamicForm.selectValue")}</option>
                <option value="true">{t("dynamicForm.true")}</option>
                <option value="false">{t("dynamicForm.false")}</option>
              </select>
            ) : (
              <input
                id={`field-${input.name}`}
                type="text"
                placeholder={t("dynamicForm.placeholderDefault")}
                value={fieldValue(input.name)}
                onChange={(e) => handleChange(input.name, input.type, e.target.value)}
                required={!input.optional}
                disabled={loading}
                style={inputStyle(input.name)}
              />
            )}

            {/* ── Inline validation error ── */}
            {fieldErrors[input.name] && (
              <span
                id={`error-${input.name}`}
                role="alert"
                style={{
                  fontSize: '12px',
                  color: '#f85149',
                  marginTop: '2px',
                }}
              >
                {fieldErrors[input.name]}
              </span>
            )}
          </div>
        ))
      )}
      <div style={{ display: 'flex', gap: '12px', marginTop: '8px' }}>
        <button
          type="submit"
          disabled={loading || hasErrors}
          style={{
            padding: '10px 20px',
            backgroundColor: loading || hasErrors ? '#30363d' : '#00d9ff',
            color: loading || hasErrors ? '#8b949e' : '#0f1117',
            border: 'none',
            borderRadius: '6px',
            fontSize: '14px',
            fontWeight: '600',
            cursor: loading || hasErrors ? 'not-allowed' : 'pointer',
            flex: 1,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            gap: '8px',
          }}
        >
          {loading ? (
            <>
              <Loader2 size={16} className="animate-spin" />
              <span>{t("dynamicForm.simulating")}</span>
            </>
          ) : (
            t("dynamicForm.simulate")
          )}
        </button>
        <button
          type="button"
          disabled={loading || hasErrors}
          style={{
            padding: '10px 20px',
            backgroundColor: loading || hasErrors ? '#30363d' : '#a371f7',
            color: loading || hasErrors ? '#8b949e' : '#fff',
            border: 'none',
            borderRadius: '6px',
            fontSize: '14px',
            fontWeight: '600',
            cursor: loading || hasErrors ? 'not-allowed' : 'pointer',
            flex: 1,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            gap: '8px',
          }}
        >
          {loading ? (
            <>
              <Loader2 size={16} className="animate-spin" />
              <span>{t("dynamicForm.invoking")}</span>
            </>
          ) : (
            t("dynamicForm.liveInvoke")
          )}
        </button>
      </div>
    </form>
  );
}
