import type { Meta, StoryObj } from "@storybook/react";
import { Spinner } from "./ui/Spinner";

/**
 * Stories for the Spinner — issue #706.
 *
 * The point of these is the reduced-motion and non-motion-cue contract from
 * #702. A spinner is the component most likely to be the *only* progress
 * signal in a view, so the stories pair every loading state with text and
 * include the reduced-motion and high-contrast variants, which are the two
 * environments where a loading indicator is most often broken.
 */
const meta = {
  title: "Components/Spinner",
  component: Spinner,
  argTypes: {
    size: { control: "select", options: ["xs", "sm", "md", "lg", "xl"] },
    color: {
      control: "select",
      options: ["primary", "white", "muted", "danger", "success"],
    },
  },
  args: { size: "md", color: "primary" },
} satisfies Meta<typeof Spinner>;

export default meta;
type Story = StoryObj<typeof meta>;

/**
 * Decorative: `aria-hidden`, no announcement.
 *
 * Only correct inside a control that already has a name ("Saving…"). Used on
 * its own it announces nothing at all, which is the point.
 */
export const Decorative: Story = {
  args: {},
};

/**
 * With a label: `role="status"`, announced politely.
 *
 * This is the correct default. The ring must never be the only progress
 * signal — under `prefers-reduced-motion` the CSS keeps it turning at 3s
 * rather than stopping it, because a stopped ring is indistinguishable from a
 * broken UI.
 */
export const WithLabel: Story = {
  args: { label: "Loading vault data…" },
};

/**
 * Reduced motion + high contrast via the global toolbar toggles.
 *
 * Storybook cannot trigger `prefers-reduced-motion` from a control, so
 * `preview.tsx` exposes it as a global and sets `data-contrast` for #701. The
 * combination matters: in high contrast the ring's low-opacity border track
 * disappears entirely, which is what `high-contrast.css` fixes.
 */
export const ReducedMotionHighContrast: Story = {
  args: { label: "Analysing contract…" },
  parameters: { globals: { reducedMotion: true, contrast: "high" } },
};

/** Status colours, to check the danger/success pair do not converge in high contrast. */
export const StatusColors: Story = {
  render: (args) => (
    <div className="flex items-center gap-6">
      <Spinner {...args} color="success" label="Uploaded" />
      <Spinner {...args} color="danger" label="Rejected" />
      <Spinner {...args} color="muted" label="Cached" />
    </div>
  ),
};

/**
 * The misuse worth showing: a bare ring beside a button that has no name.
 *
 * Documented as a story so it cannot be copied: `button-name` fails in the
 * a11y panel, and the ring contributes nothing because it is `aria-hidden`.
 */
export const UnlabelledButtonAntiPattern: Story = {
  render: () => (
    <button type="button">
      <Spinner size="sm" />
    </button>
  ),
  parameters: {
    docs: {
      description: {
        story: "Fails `button-name`. Do not copy.",
      },
    },
  },
};