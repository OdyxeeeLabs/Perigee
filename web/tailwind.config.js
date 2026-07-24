/** @type {import('tailwindcss').Config} */
const colors = require('tailwindcss/colors');

module.exports = {
  content: [
    "./pages/**/*.{js,ts,jsx,tsx}",
    "./components/**/*.{js,ts,jsx,tsx}",
    "./context/**/*.{js,ts,jsx,tsx}",
    "./wasm-upload/**/*.{js,ts,jsx,tsx}",
  ],
  // Status/severity indicators (CPU, RAM, ledger budgets, call-graph hotspots)
  // pick their color classes from a fixed palette at runtime. Never interpolate
  // these into template strings (e.g. `text-${color}-400`) - Tailwind's
  // production purge can only keep classes it finds as complete literal
  // strings in scanned files, so this safelist guards the palette even if a
  // future refactor moves the lookup logic somewhere the literals aren't visible.
  safelist: [
    {
      pattern:
        /^(bg|text|border|ring)-(rose|amber|emerald|cyan|sky|violet|pink|indigo|orange|blue|green|slate)-(50|100|200|300|400|500|600|700|800|900|950)$/,
    },
  ],
  theme: {
    extend: {
      colors: {
        slate: {
          ...colors.slate,
          // Override 400 and 500 to be lighter for WCAG AA compliance against bg-slate-950
          400: colors.slate[300], // #cbd5e1
          500: colors.slate[400], // #94a3b8
        },
        gray: {
          ...colors.gray,
          400: colors.gray[300], // #d1d5db
          500: colors.gray[400], // #9ca3af
        }
      },
      spacing: {
        120: "30rem",
      },
      borderRadius: {
        "4xl": "2rem",
        "s-2xl": "1rem 0 0 1rem",
        "e-2xl": "0 1rem 1rem 0",
      },
    },
  },
  plugins: [],
};
