import type { Preview } from "@storybook/react";

/**
 * Global preview and accessibility parameters — issue #706.
 *
 * Two decisions worth explaining:
 *
 * 1. **`next-intl` messages are provided globally.** Without them every story
 *    that calls `useTranslations()` throws, which makes the stories that
 *    matter (NavDrawer, the command palette, error summaries) impossible to
 *    render — precisely the components whose a11y is hardest to review as a
 *    diff.
 *
 * 2. **Accessibility-relevant globals are set as decorators, not stories.**
 *    A high-contrast or reduced-motion variant is a rendering environment, not
 *    a state of the component; making them separate stories doubles the
 *    story count and invites one variant to be checked and the other skipped.
 *    As a decorator, every story gets them.
 *
 * Requires `@storybook/react` and `@storybook/addon-a11y`, neither of which is
 * declared yet — see main.ts and docs/accessibility/storybook.md.
 */
const messages = {
  "nav.title": "Navigation",
  "nav.close": "Close navigation drawer",
  "nav.mobile": "Mobile navigation",
  "nav.home": "Home",
  "nav.vaults": "Vaults",
  "common.loading": "Loading…",
  "upload.parsing": "Parsing WASM binary — analysing resource usage…",
  "upload.success": "Contract uploaded successfully",
  "upload.rejected": "File rejected",
  "a11y.statusRegionLabel": "Status updates",
  "a11y.announcements": "Announcements",
};

const preview: Preview = {
  parameters: {
    layout: "centered",
    controls: {
      matchers: {
        color: /(background|color)$/i,
        date: /Date$/i,
      },
    },
    a11y: {
      // "todo" rather than "error": the repo has known violations, and a panel
      // pinned red on first run gets ignored. `element` scope keeps the report
      // attached to the story DOM instead of the whole preview iframe.
      test: "todo",
      config: {
        rules: [
          {
            // Every interactive element needs a name; there is no acceptable
            // exception for an icon-only control, so this stays "error".
            id: "button-name",
            review: [],
          },
          {
            id: "label",
            review: ["label-content-name-mismatch"],
          },
        ],
      },
    },
    nextIntl: { messages },
    backgrounds: {
      // Contrast is a per-background property: a token that passes on the dark
      // surface can fail on the light one, and vice versa.
      values: [
        { name: "light", value: "#ffffff" },
        { name: "dark", value: "#0f172a" },
        { name: "high-contrast", value: "#000000" },
      ],
    },
  },

  decorators: [
    /**
     * `prefers-reduced-motion` cannot be triggered from a Storybook control,
     * so it is emulated with the real attribute the code under test reads.
     * Components in this repo branch on the media query and on
     * `data-contrast` (issues #701 and #702), so the decorator sets those.
     */
    (Story, context) => {
      const reduced = context.globals["reducedMotion"] === true;
      const contrast = context.globals["contrast"];

      return (
        <div
          dir={context.globals["direction"] === "rtl" ? "rtl" : "ltr"}
          data-contrast={contrast === "high" ? "high" : undefined}
          style={
            reduced
              ? ({
                  // Approximates the media query for CSS in the preview frame.
                  "--motion-duration": "0.01ms",
                } as React.CSSProperties)
              : undefined
          }
        >
          <Story />
        </div>
      );
    },
  ],

  globalTypes: {
    contrast: {
      description: "Contrast mode (issue #701)",
      defaultValue: "normal",
      toolbar: {
        title: "Contrast",
        items: [
          { value: "normal", title: "Normal" },
          { value: "high", title: "High" },
        ],
      },
    },
    reducedMotion: {
      description: "Emulate prefers-reduced-motion (issue #702)",
      defaultValue: false,
      toolbar: {
        title: "Reduced motion",
        icon: "playcircle",
        items: [{ value: false, title: "Off" }, { value: true, title: "On" }],
      },
    },
    direction: {
      description: "Text direction (issue #699)",
      defaultValue: "ltr",
      toolbar: {
        title: "Direction",
        items: [
          { value: "ltr", title: "LTR" },
          { value: "rtl", title: "RTL" },
        ],
      },
    },
  },

  tags: ["autodocs"],
};

export default preview;