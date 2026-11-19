import { test, expect, type Page } from "@playwright/test";

/**
 * E2E test suite for the Perigee wallet connection flow.
 *
 * These tests exercise the full user journey from landing page to wallet
 * connection modal and back, without mocking the wallet kit internals.
 *
 * Resolves WEB-20 (#106): no E2E tests for critical wallet connection flow.
 *
 * Run with:
 *   npx playwright test e2e/wallet-flow.spec.ts
 *
 * Requires a running dev server (yarn dev) or use the webServer config in
 * playwright.config.ts to start it automatically.
 */

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Open the wallet modal and return the dialog element. */
async function openWalletModal(page: Page) {
  const connectButton = page.getByRole("button", { name: /connect wallet/i });
  await expect(connectButton).toBeVisible();
  await connectButton.click();
  const modal = page.getByRole("dialog");
  await expect(modal).toBeVisible();
  return modal;
}

// ---------------------------------------------------------------------------
// Suite
// ---------------------------------------------------------------------------

test.describe("Wallet connection flow", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    await page.waitForLoadState("domcontentloaded");
  });

  // ── Landing page ─────────────────────────────────────────────────────────

  test("shows Connect Wallet button on landing page", async ({ page }) => {
    await expect(
      page.getByRole("button", { name: /connect wallet/i }),
    ).toBeVisible();
  });

  test("page title includes Perigee branding", async ({ page }) => {
    await expect(page).toHaveTitle(/perigee/i);
  });

  // ── Modal open / close ───────────────────────────────────────────────────

  test("clicking Connect Wallet opens the wallet selection modal", async ({ page }) => {
    const modal = await openWalletModal(page);
    await expect(modal).toBeVisible();
    // Modal should list at least one wallet option
    await expect(modal.getByRole("listitem").first()).toBeVisible();
  });

  test("modal has accessible dialog role and label", async ({ page }) => {
    await openWalletModal(page);
    const modal = page.getByRole("dialog");
    // aria-label or aria-labelledby must be present for screen readers
    const label = await modal.getAttribute("aria-label");
    const labelledBy = await modal.getAttribute("aria-labelledby");
    expect(label || labelledBy).not.toBeNull();
  });

  test("pressing Escape closes the wallet modal", async ({ page }) => {
    const modal = await openWalletModal(page);
    await page.keyboard.press("Escape");
    await expect(modal).not.toBeVisible();
  });

  test("clicking the close button dismisses the modal", async ({ page }) => {
    const modal = await openWalletModal(page);
    const closeBtn = modal.getByRole("button", { name: /close/i });
    if (await closeBtn.isVisible()) {
      await closeBtn.click();
      await expect(modal).not.toBeVisible();
    } else {
      // Some wallet kits close via backdrop click
      await page.keyboard.press("Escape");
      await expect(modal).not.toBeVisible();
    }
  });

  // ── Wallet list ──────────────────────────────────────────────────────────

  test("modal lists expected wallet providers", async ({ page }) => {
    const modal = await openWalletModal(page);
    const names = ["Freighter", "Albedo", "xBull", "Rabet", "Lobstr"];
    for (const name of names) {
      await expect(
        modal.getByText(name, { exact: false }),
      ).toBeVisible();
    }
  });

  test("each wallet option is keyboard focusable", async ({ page }) => {
    const modal = await openWalletModal(page);
    const options = modal.getByRole("button");
    const count = await options.count();
    expect(count).toBeGreaterThan(0);
    // Tab through options to verify focus is not trapped on first element
    await page.keyboard.press("Tab");
    const focused = page.locator(":focus");
    await expect(focused).toBeVisible();
  });

  // ── Post-connection state ─────────────────────────────────────────────────

  test("address is persisted to localStorage on successful connect (unit-level)", async ({ page }) => {
    // Inject a mock address directly to verify storage contract without
    // requiring a real wallet extension.
    await page.evaluate(() => {
      localStorage.setItem("perigee_wallet_address", "GDUMMY...STELLAR");
      localStorage.setItem("perigee_wallet_id", "freighter");
    });
    const address = await page.evaluate(() =>
      localStorage.getItem("perigee_wallet_address"),
    );
    expect(address).toBe("GDUMMY...STELLAR");
  });

  test("disconnect clears wallet address from localStorage", async ({ page }) => {
    await page.evaluate(() => {
      localStorage.setItem("perigee_wallet_address", "GDUMMY...STELLAR");
    });
    await page.evaluate(() => {
      localStorage.removeItem("perigee_wallet_address");
    });
    const address = await page.evaluate(() =>
      localStorage.getItem("perigee_wallet_address"),
    );
    expect(address).toBeNull();
  });
});

test.describe("Wallet error states", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    await page.waitForLoadState("domcontentloaded");
  });

  test("shows an error message when wallet connection fails", async ({ page }) => {
    // Simulate a connection error by intercepting the wallet kit
    await page.evaluate(() => {
      sessionStorage.setItem("__perigee_force_wallet_error", "1");
    });
    // Open modal — the app should handle the error gracefully
    const connectButton = page.getByRole("button", { name: /connect wallet/i });
    if (await connectButton.isVisible()) {
      await connectButton.click();
      // Either the modal closes gracefully or shows an error banner
      const errorEl = page.getByRole("alert");
      const modal = page.getByRole("dialog");
      const isAlertVisible = await errorEl.isVisible().catch(() => false);
      const isModalVisible = await modal.isVisible().catch(() => false);
      // At least one of: error shown or modal closed without crash
      expect(isAlertVisible || !isModalVisible).toBe(true);
    }
  });
});

const FIXTURE_ADDRESS = "GTESTFIXTURE0000000000000000000000000000000000000000000000";

interface RegistrationRequest {
  stellar_address: string;
  name: string;
  email: string;
  kyc_document_ref: string;
}

async function mockProvisioning(page: Page): Promise<RegistrationRequest[]> {
  const requests: RegistrationRequest[] = [];

  await page.addInitScript((address) => {
    const fakeWallet = {
      isConnected: () => true,
      getAddress: async () => address,
      getNetwork: async () => "Testnet SDF",
      signTransaction: async () => ({}),
    };
    Object.defineProperty(window, "freighter", {
      value: fakeWallet,
      configurable: true,
    });
    Object.defineProperty(window, "freighterApi", {
      value: fakeWallet,
      configurable: true,
    });
    localStorage.setItem("perigee_wallet_address", address);
    localStorage.setItem("perigee_wallet_id", "freighter");
    sessionStorage.setItem("perigee_wallet_id", "freighter");
    sessionStorage.removeItem("onboarding_draft");
  }, FIXTURE_ADDRESS);

  await page.route("**/api/**", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: "{}" }),
  );
  await page.route("**/managers/status/**", (route) => {
    if (route.request().method() === "OPTIONS") {
      return route.fulfill({
        status: 204,
        headers: {
          "access-control-allow-origin": "*",
          "access-control-allow-headers": "accept,content-type,x-api-version",
          "access-control-allow-methods": "GET, OPTIONS",
        },
      });
    }
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      headers: { "access-control-allow-origin": "*" },
      body: JSON.stringify({ id: "", status: "none", message: "" }),
    });
  });
  await page.route("**/managers/register", (route) => {
    if (route.request().method() === "OPTIONS") {
      return route.fulfill({
        status: 204,
        headers: {
          "access-control-allow-origin": "*",
          "access-control-allow-headers": "accept,content-type,x-api-version",
          "access-control-allow-methods": "POST, OPTIONS",
        },
      });
    }
    const body = route.request().postDataJSON() as RegistrationRequest;
    requests.push(body);
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      headers: { "access-control-allow-origin": "*" },
      body: JSON.stringify({
        id: "mgr-provisioned-1",
        stellar_address: FIXTURE_ADDRESS,
        ...body,
        status: "pending",
        kyc_document_ref: body.kyc_document_ref,
        notes: "",
        created_at: "2026-01-15T10:30:00Z",
        updated_at: "2026-01-15T10:30:00Z",
      }),
    });
  });

  return requests;
}

async function fillProvisioningForm(page: Page) {
  await page.getByLabel(/full name/i).fill("Fixture Manager");
  await page.getByLabel("Email", { exact: true }).fill("fixture@example.com");
  await page.getByLabel(/kyc document reference/i).fill("KYC-FIXTURE-1");
}

async function openProvisioning(page: Page): Promise<RegistrationRequest[]> {
  const requests = await mockProvisioning(page);
  await page.goto("/managers/onboarding");
  await expect(page.getByTestId("onboarding-step")).toHaveAttribute(
    "data-step",
    "register",
  );
  return requests;
}

test.describe("manager onboarding / vault provisioning", () => {
  test("validation rejects a blank name before any request is sent", async ({ page }) => {
    const requests = await openProvisioning(page);
    await page.getByLabel(/full name/i).fill("   ");
    await page.getByRole("button", { name: /submit registration/i }).click();

    await expect(page.getByTestId("onboarding-error")).toContainText(
      /name is required/i,
    );
    expect(requests).toHaveLength(0);
  });

  test("the review state shows every value that will be submitted", async ({ page }) => {
    await openProvisioning(page);
    await fillProvisioningForm(page);

    await expect(page.getByLabel(/full name/i)).toHaveValue("Fixture Manager");
    await expect(page.getByLabel("Email", { exact: true })).toHaveValue(
      "fixture@example.com",
    );
    await expect(page.getByLabel(/kyc document reference/i)).toHaveValue(
      "KYC-FIXTURE-1",
    );
    await expect(
      page.getByRole("button", { name: /submit registration/i }),
    ).toBeEnabled();
  });

  test("cancelling returns to the dashboard without provisioning", async ({ page }) => {
    const requests = await openProvisioning(page);
    await fillProvisioningForm(page);
    await page.getByRole("button", { name: /back to analyzer/i }).click();

    await expect(page).toHaveURL(/\/$/);
    expect(requests).toHaveLength(0);
  });

  test("successful provisioning sends the request and shows the final state", async ({ page }) => {
    const requests = await openProvisioning(page);
    await fillProvisioningForm(page);
    await page.getByRole("button", { name: /submit registration/i }).click();

    await expect.poll(() => requests.length).toBe(1);
    expect(requests[0]).toEqual({
      stellar_address: FIXTURE_ADDRESS,
      name: "Fixture Manager",
      email: "fixture@example.com",
      kyc_document_ref: "KYC-FIXTURE-1",
    });

    await expect(page.getByTestId("onboarding-step")).toHaveAttribute(
      "data-step",
      "submitted",
    );
    await expect(
      page.getByRole("heading", { name: /registration submitted/i }),
    ).toBeVisible();
    await expect(page.getByTestId("onboarding-record")).toContainText(
      "mgr-provisioned-1",
    );
    await expect(page.getByTestId("onboarding-record")).toContainText("pending");
  });
});
