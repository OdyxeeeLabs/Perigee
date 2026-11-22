"use client";

export const PRIVACY_NOTICE_DISMISSED_KEY = "perigee_privacy_notice_dismissed";

export interface PrivacyNoticeStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

function getStorage(): PrivacyNoticeStorage | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export function isPrivacyNoticeDismissed(
  storage: PrivacyNoticeStorage | null = getStorage(),
): boolean {
  try {
    return storage?.getItem(PRIVACY_NOTICE_DISMISSED_KEY) === "1";
  } catch {
    return false;
  }
}

export function dismissPrivacyNotice(
  storage: PrivacyNoticeStorage | null = getStorage(),
): void {
  try {
    storage?.setItem(PRIVACY_NOTICE_DISMISSED_KEY, "1");
  } catch {
    return;
  }
}
