// release.config.js
// Configuration for conventional-changelog-cli.
// Run via: yarn changelog
//
// This writes/updates CHANGELOG.md from git history following
// the conventionalcommits preset, which honours BREAKING CHANGE footers
// and maps types to changelog sections.

/** @type {import('conventional-changelog-config-spec').Config} */
export default {
  // Use the same preset as commitlint so the two configs stay in sync.
  preset: {
    name: 'conventionalcommits',

    // Map commit types to changelog sections.
    // Types not listed here are omitted from the changelog.
    types: [
      { type: 'feat',     section: '✨ Features' },
      { type: 'fix',      section: '🐛 Bug Fixes' },
      { type: 'perf',     section: '⚡ Performance' },
      { type: 'revert',   section: '⏪ Reverts' },
      { type: 'docs',     section: '📝 Documentation',  hidden: false },
      { type: 'refactor', section: '♻️  Refactors',       hidden: false },
      { type: 'contract', section: '📜 Contracts',       hidden: false },
      { type: 'agent',    section: '🤖 Agent Fleet',     hidden: false },
      { type: 'build',    section: '🏗️  Build',           hidden: true  },
      { type: 'ci',       section: '👷 CI',              hidden: true  },
      { type: 'chore',    section: '🔧 Chores',          hidden: true  },
      { type: 'style',    hidden: true },
      { type: 'test',     hidden: true },
    ],
  },
};
