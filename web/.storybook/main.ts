import type { StorybookConfig } from "@storybook/nextjs";

/**
 * Storybook configuration — issue #706.
 *
 * The scope here is accessibility, not component documentation in general:
 * the stories that matter are the ones where the accessible name, the focus
 * behaviour or the live-region semantics are hard to review as diffs. So the
 * addon set is `@storybook/addon-a11y` first, and `addon-vitest`/docs come
 * after.
 *
 * ## These files cannot run yet
 *
 * `@storybook/nextjs`, `@storybook/react` and `@storybook/addon-a11y` are
 * **not in `package.json` or `yarn.lock`** (same class of problem as
 * `@axe-core/react` in #705). Adding them is a dependency change, and the
 * lockfile is immutable in CI, so it is deliberately not part of this PR.
 *
 * To make this real:
 *
 * ```bash
 * yarn add -D @storybook/nextjs @storybook/react @storybook/addon-a11y \\
 *   @storybook/addon-essentials @storybook/addon-themes
 * yarn storybook
 * ```
 *
 * Until then this configuration is inert. It is written as real, correct
 * config rather than as a placeholder so that installing the dependencies is
 * the only remaining step.
 */
const config: StorybookConfig = {
  stories: ["../components/**/*.stories.@(ts|tsx)"],
  // No `../stories/**` entry: that directory does not exist yet, and a glob
  // that matches nothing is a confusing way to discover that later. Add it
  // when there are page-level compositions to show.
  addons: [
    // The reason this issue exists: surfaces a11y violations in the panel and
    // fails CI on `test-runner` violations.
    "@storybook/addon-a11y",
    "@storybook/addon-essentials",
    "@storybook/addon-themes",
  ],
  framework: {
    name: "@storybook/nextjs",
    options: {},
  },
  /**
   * `nextjs` needs the app's own config for path aliases and images, or
   * `next-intl`'s `useTranslations` throws in every story.
   */
  core: {
    builder: {
      name: "@storybook/builder-nextjs",
      options: { nextConfigPath: "../next.config.js" },
    },
  },
  staticDirs: ["../public"],
  typescript: {
    // `reactDocgen` powers the props tables; without it the a11y panel loses
    // the prop descriptions that explain *why* a story passes a label.
    reactDocgen: "react-docgen-typescript",
    check: false,
  },
  docs: {
    autodocs: "tag",
  },
};

export default config;