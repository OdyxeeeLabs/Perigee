import { test, expect, type Page } from '@playwright/test';
import {
  testWallets,
  testVaults,
  testStrategies,
  testSettings,
} from '../fixtures/testData';

async function seedWalletConnection(page: Page, wallet: { address: string }) {
  await page.addInitScript((address) => {
    window.localStorage.setItem('wallet:connected', address);
  }, wallet.address);
}

class WalletPage {
  constructor(readonly page: Page) {}

  connectButton() {
    return this.page.getByRole('button', { name: /connect wallet/i });
  }

  walletOption(name: string) {
    return this.page.getByRole('button', { name });
  }

  connectedAddress() {
    return this.page.getByTestId('wallet-address');
  }

  async connect(walletName: string) {
    await this.connectButton().click();
    await this.walletOption(walletName).click();
  }
}

class VaultPage {
  constructor(readonly page: Page) {}

  createButton() {
    return this.page.getByTestId('create-vault');
  }

  nameInput() {
    return this.page.getByLabel('Vault name');
  }

  assetSelect() {
    return this.page.getByLabel('Asset');
  }

  depositInput() {
    return this.page.getByLabel('Deposit amount');
  }

  withdrawInput() {
    return this.page.getByLabel('Withdraw amount');
  }

  strategySelect() {
    return this.page.getByLabel('Strategy');
  }

  submitButton() {
    return this.page.getByRole('button', { name: /create vault/i });
  }

  depositButton() {
    return this.page.getByTestId('deposit');
  }

  withdrawButton() {
    return this.page.getByTestId('withdraw');
  }

  vaultCard(name: string) {
    return this.page.getByTestId(`vault-${name}`);
  }

  async createVault(name: string, asset: string, amount: string) {
    await this.createButton().click();
    await this.nameInput().fill(name);
    await this.assetSelect().selectOption(asset);
    await this.depositInput().fill(amount);
    await this.submitButton().click();
  }

  async deposit(amount: string) {
    await this.depositInput().fill(amount);
    await this.depositButton().click();
  }

  async withdraw(amount: string) {
    await this.withdrawInput().fill(amount);
    await this.withdrawButton().click();
  }

  async switchStrategy(strategy: string) {
    await this.strategySelect().selectOption(strategy);
  }
}

class PortfolioPage {
  constructor(readonly page: Page) {}

  totalValue() {
    return this.page.getByTestId('portfolio-total-value');
  }

  positionRow(asset: string) {
    return this.page.getByTestId(`position-${asset}`);
  }

  async goto() {
    await this.page.getByRole('link', { name: /portfolio/i }).click();
  }
}

class SettingsPage {
  constructor(readonly page: Page) {}

  themeToggle() {
    return this.page.getByTestId('theme-toggle');
  }

  currencySelect() {
    return this.page.getByLabel('Currency');
  }

  notificationsToggle() {
    return this.page.getByTestId('notifications-toggle');
  }

  saveButton() {
    return this.page.getByTestId('save-settings');
  }

  async goto() {
    await this.page.getByRole('link', { name: /settings/i }).click();
  }

  async updateSettings(theme: string, currency: string, notifications: boolean) {
    const currentTheme = await this.themeToggle().getAttribute('aria-checked');
    if ((currentTheme === 'true') !== (theme === 'dark')) {
      await this.themeToggle().click();
    }
    await this.currencySelect().selectOption(currency);
    const notificationsChecked = await this.notificationsToggle().getAttribute('aria-checked');
    if ((notificationsChecked === 'true') !== notifications) {
      await this.notificationsToggle().click();
    }
    await this.saveButton().click();
  }
}

test.describe('Wallet connection', () => {
  test('connects a wallet and shows the address', async ({ page }) => {
    const wallet = testWallets.freighter;
    await page.goto('/');
    const walletPage = new WalletPage(page);
    await walletPage.connect(wallet.name);
    await expect(walletPage.connectedAddress()).toBeVisible();
    await expect(walletPage.connectedAddress()).toContainText(wallet.address.slice(0, 6));
  });

  test('persists connection across reloads', async ({ page }) => {
    const wallet = testWallets.lobstr;
    await seedWalletConnection(page, wallet);
    await page.goto('/');
    await page.reload();
    const walletPage = new WalletPage(page);
    await expect(walletPage.connectedAddress()).toBeVisible();
  });
});

test.describe('Vault creation', () => {
  test.beforeEach(async ({ page }) => {
    await seedWalletConnection(page, testWallets.freighter);
    await page.goto('/');
  });

  test('creates a new vault', async ({ page }) => {
    const vault = testVaults.default;
    const vaultPage = new VaultPage(page);
    await vaultPage.createVault(vault.name, vault.asset, vault.depositAmount);
    await expect(vaultPage.vaultCard(vault.name)).toBeVisible();
  });

  test('validates required fields', async ({ page }) => {
    const vaultPage = new VaultPage(page);
    await vaultPage.createButton().click();
    await vaultPage.submitButton().click();
    await expect(page.getByText(/required/i)).toBeVisible();
  });
});

test.describe('Deposit and withdrawal', () => {
  test.beforeEach(async ({ page }) => {
    await seedWalletConnection(page, testWallets.freighter);
    await page.goto('/');
    const vaultPage = new VaultPage(page);
    const vault = testVaults.default;
    await vaultPage.createVault(vault.name, vault.asset, vault.depositAmount);
  });

  test('deposits assets into a vault', async ({ page }) => {
    const vaultPage = new VaultPage(page);
    const vault = testVaults.default;
    await vaultPage.deposit(vault.depositAmount);
    await expect(page.getByTestId('deposit-success')).toBeVisible();
  });

  test('withdraws assets from a vault', async ({ page }) => {
    const vaultPage = new VaultPage(page);
    const vault = testVaults.default;
    await vaultPage.withdraw(vault.withdrawAmount);
    await expect(page.getByTestId('withdraw-success')).toBeVisible();
  });

  test('prevents withdrawing more than the balance', async ({ page }) => {
    const vaultPage = new VaultPage(page);
    await vaultPage.withdraw('999999');
    await expect(page.getByText(/insufficient/i)).toBeVisible();
  });
});

test.describe('Strategy switch', () => {
  test.beforeEach(async ({ page }) => {
    await seedWalletConnection(page, testWallets.freighter);
    await page.goto('/');
    const vaultPage = new VaultPage(page);
    const vault = testVaults.default;
    await vaultPage.createVault(vault.name, vault.asset, vault.depositAmount);
  });

  for (const strategy of testStrategies) {
    test(`switches to ${strategy.name} strategy`, async ({ page }) => {
      const vaultPage = new VaultPage(page);
      await vaultPage.switchStrategy(strategy.id);
      await expect(page.getByTestId(`strategy-${strategy.id}-selected`)).toBeVisible();
    });
  }
});

test.describe('Portfolio view', () => {
  test.beforeEach(async ({ page }) => {
    await seedWalletConnection(page, testWallets.freighter);
    await page.goto('/');
  });

  test('shows total portfolio value', async ({ page }) => {
    const portfolioPage = new PortfolioPage(page);
    await portfolioPage.goto();
    await expect(portfolioPage.totalValue()).toBeVisible();
  });

  test('lists individual positions', async ({ page }) => {
    const portfolioPage = new PortfolioPage(page);
    await portfolioPage.goto();
    await expect(portfolioPage.positionRow(testVaults.default.asset)).toBeVisible();
  });
});

test.describe('Settings changes', () => {
  test.beforeEach(async ({ page }) => {
    await seedWalletConnection(page, testWallets.freighter);
    await page.goto('/');
  });

  test('updates theme, currency and notifications', async ({ page }) => {
    const settingsPage = new SettingsPage(page);
    await settingsPage.goto();
    await settingsPage.updateSettings(
      testSettings.theme,
      testSettings.currency,
      testSettings.notifications
    );
    await expect(page.getByTestId('settings-saved')).toBeVisible();
  });

  test('persists settings across reloads', async ({ page }) => {
    const settingsPage = new SettingsPage(page);
    await settingsPage.goto();
    await settingsPage.updateSettings(
      testSettings.theme,
      testSettings.currency,
      testSettings.notifications
    );
    await page.reload();
    await expect(settingsPage.currencySelect()).toHasValue(testSettings.currency);
  });
});
