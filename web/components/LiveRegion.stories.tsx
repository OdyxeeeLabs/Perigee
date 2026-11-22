import type { Meta, StoryObj } from "@storybook/react";
import { LiveAlert, LiveStatus } from "./LiveRegion";

/**
 * Stories for the shared live regions — issue #706.
 *
 * These exist because a live region's accessibility is not visible in a diff:
 * the markup for a correct polite region and an incorrect one is nearly
 * identical, and the difference is whether the region was already in the DOM
 * when the message arrived. Rendering them side by side with axe running in
 * the panel is the cheapest way to review that.
 */
const meta = {
  title: "Accessibility/LiveRegion",
  component: LiveStatus,
  parameters: {
    docs: {
      description: {
        component:
          "Both regions must stay mounted and only change their message. A region " +
          "that mounts together with its text is frequently never announced, because " +
          "assistive technology has not registered it yet.",
      },
    },
  },
} satisfies Meta<typeof LiveStatus>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Empty and mounted: the state a live region must be in before its first message. */
export const Idle: Story = {
  args: { message: "", label: "Status updates" },
};

/** Polite: progress and completion queue behind whatever the user is doing. */
export const AnalysisRunning: Story = {
  args: { message: "Analysis running for 2 minutes", label: "Status updates" },
};

/** Polite: completion is news, but not urgent news. */
export const AnalysisComplete: Story = {
  args: { message: "Analysis complete", label: "Status updates" },
};

/**
 * Assertive — interrupts, because the user has to act on it.
 *
 * Kept separate from the polite stories deliberately: mixing the two in one
 * story is how a "Saved!" confirmation ends up interrupting a screen reader
 * mid-sentence.
 */
export const AnalysisFailed: Story = {
  render: () => <LiveAlert message="Analysis failed: insufficient fee" />,
};

/** Both together, as `_app.tsx` mounts them: two regions, one page. */
export const Paired: Story = {
  render: () => (
    <>
      <LiveStatus message="Contract uploaded successfully" label="Status updates" />
      <LiveAlert message="Gas estimate failed" />
    </>
  ),
};