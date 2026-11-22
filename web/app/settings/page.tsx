import type { Metadata } from "next";
import { SettingsIntlProvider } from "@/components/settings/SettingsIntlProvider";
import { SettingsPage } from "@/components/settings/SettingsPage";

export const metadata: Metadata = {
  title: "Settings | Perigee",
  description: "Configure network, trading, display, privacy, security and advanced preferences.",
};

export default function Settings() {
  return (
    <SettingsIntlProvider>
      <SettingsPage />
    </SettingsIntlProvider>
  );
}
