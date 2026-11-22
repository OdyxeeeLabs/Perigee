/**
 * Toast notification sounds — issue #681.
 *
 * Generates short, non-intrusive cues with the Web Audio API instead of
 * shipping binary audio assets. Each variant maps to a distinct pitch so the
 * ear can tell "success" from "error" at a glance (or rather, at a listen).
 *
 * Sound is entirely opt-in: the toast store only calls `playToastSound` when
 * the user has enabled it, and every entry point is safe to call during
 * server rendering or in browsers without `AudioContext` (it no-ops).
 */

import type { ToastVariant } from "../context/ToastContext";

/** Base frequency (Hz) per variant — a small major-ish motif for positives,
 *  a low descending cue for errors. */
const TONE_BY_VARIANT: Record<ToastVariant, { freq: number; type: OscillatorType }> = {
  success: { freq: 880, type: "sine" },
  info: { freq: 660, type: "sine" },
  warning: { freq: 523, type: "triangle" },
  error: { freq: 294, type: "sawtooth" },
};

let audioContext: AudioContext | null = null;

function getContext(): AudioContext | null {
  if (typeof window === "undefined") return null;
  const Ctor = window.AudioContext;
  if (!Ctor) return null;
  if (!audioContext) audioContext = new Ctor();
  return audioContext;
}

/**
 * Plays a brief tone for the given variant.
 *
 * Fails silently when audio is unavailable or the context is blocked (e.g.
 * autoplay policy before the first user gesture) — a missing beep must never
 * break the notification itself.
 */
export function playToastSound(variant: ToastVariant): void {
  const ctx = getContext();
  if (!ctx) return;

  try {
    // Browsers keep the context suspended until a user gesture resumes it.
    if (ctx.state === "suspended") {
      void ctx.resume();
    }

    const { freq, type } = TONE_BY_VARIANT[variant];
    const start = ctx.currentTime;
    const duration = variant === "error" ? 0.22 : 0.14;

    const oscillator = ctx.createOscillator();
    const gain = ctx.createGain();

    oscillator.type = type;
    oscillator.frequency.setValueAtTime(freq, start);

    // Quick attack, gentle decay — avoids a harsh click.
    gain.gain.setValueAtTime(0.0001, start);
    gain.gain.exponentialRampToValueAtTime(0.12, start + 0.02);
    gain.gain.exponentialRampToValueAtTime(0.0001, start + duration);

    oscillator.connect(gain);
    gain.connect(ctx.destination);

    oscillator.start(start);
    oscillator.stop(start + duration + 0.02);
  } catch {
    // Autoplay restrictions or unsupported configurations: ignore.
  }
}
