// commitlint.config.mjs
// Enforces Conventional Commits (https://www.conventionalcommits.org)
// Used by the commit-msg husky hook and the CI lint-commits workflow.

/** @type {import('@commitlint/types').UserConfig} */
export default {
  extends: ['@commitlint/config-conventional'],

  rules: {
    // Allow types used across this monorepo
    'type-enum': [
      2,
      'always',
      [
        'feat',     // new feature
        'fix',      // bug fix
        'docs',     // documentation only
        'style',    // formatting, whitespace — no logic change
        'refactor', // code change that is neither fix nor feat
        'perf',     // performance improvement
        'test',     // adding or correcting tests
        'build',    // build system / dependency changes
        'ci',       // CI configuration
        'chore',    // other maintenance
        'revert',   // reverts a previous commit
        'contract', // Soroban contract changes (project-specific)
        'agent',    // agent fleet / runtime changes (project-specific)
      ],
    ],

    // Scope must be lowercase when provided
    'scope-case': [2, 'always', 'lower-case'],

    // Subject line: sentence-case is fine, but no trailing period
    'subject-full-stop': [2, 'never', '.'],

    // Subject must not be empty
    'subject-empty': [2, 'never'],

    // Body/footer lines may be long (URLs in changelogs)
    'body-max-line-length': [1, 'always', 200],
    'footer-max-line-length': [1, 'always', 200],

    // Header max length — keeps summaries readable in `git log --oneline`
    'header-max-length': [2, 'always', 100],
  },

  // Prompt configuration for `commitlint --edit` interactive mode (optional)
  prompt: {
    settings: {},
    messages: {
      skip: '(press enter to skip)',
      max: 'upper %d chars',
      min: '%d chars at least',
      emptyWarning: 'can not be empty',
      upperLimitWarning: 'over limit',
      lowerLimitWarning: 'below limit',
    },
    questions: {
      type: {
        description: "Select the type of change you're committing",
      },
      scope: {
        description:
          'What is the scope of this change (e.g. vault, agent, web, contracts)',
      },
      subject: {
        description: 'Write a short, imperative tense description of the change',
      },
      body: {
        description: 'Provide a longer description of the change (optional)',
      },
      isBreaking: {
        description: 'Are there any breaking changes?',
      },
      breakingBody: {
        description:
          'A BREAKING CHANGE commit requires a body. Please enter a longer description',
      },
      breaking: {
        description: 'Describe the breaking changes',
      },
      isIssueAffected: {
        description: 'Does this change affect any open issues?',
      },
      issuesBody: {
        description:
          'If issues are closed, the commit requires a body. Please enter a longer description',
      },
      issues: {
        description: 'Add issue references (e.g. "fix #123", "re #456")',
      },
    },
  },
};
