"use client";

import { useCallback, useEffect, useState } from "react";
import { useTranslations } from "next-intl";
import { LiveStatus } from "../LiveRegion";
import { managerService, type ManagerRecord } from "../../lib/api";
import { clearCache } from "../../lib/apiCache";
import {
  stateFromLoadError,
  stateFromLoadResult,
  type ManagersViewState,
} from "../../lib/adminManagersState";

/**
 * Minimal token seam. The backend remains the authorization boundary; this
 * only forwards a token when one is already present in the browser session.
 */
function getAccessToken(): string | undefined {
  if (typeof window === "undefined") return undefined;
  try {
    return window.sessionStorage.getItem("perigee_access_token") ?? undefined;
  } catch {
    return undefined;
  }
}

/**
 * Admin manager list (WEB-03 / #804).
 *
 * Loading, populated, forbidden, unauthenticated and service-error states are
 * distinct. A 401 or 403 clears the shared API cache so a user who loses admin
 * access can never keep seeing previously cached manager data.
 */
export default function AdminManagersClient() {
  const t = useTranslations();
  const [state, setState] = useState<ManagersViewState>("loading");
  const [managers, setManagers] = useState<ManagerRecord[]>([]);

  const load = useCallback(async () => {
    setState("loading");
    setManagers([]);

    try {
      const page = await managerService.listAdminManagers(getAccessToken());
      const rows = page.data;
      setManagers(rows);
      setState(stateFromLoadResult(rows.length));
    } catch (error) {
      // Drop every cached read before showing an authorization failure. This
      // is what stops a former admin from seeing stale privileged data.
      clearCache();
      setManagers([]);
      setState(stateFromLoadError(error));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const statusMessage =
    state === "loading"
      ? t("admin.managers.loading")
      : state === "forbidden"
        ? t("admin.managers.forbidden")
        : state === "unauthenticated"
          ? t("admin.managers.unauthenticated")
          : state === "service-error"
            ? t("admin.managers.serviceUnavailable")
            : "";

  return (
    <div className="mt-6">
      <LiveStatus
        message={statusMessage}
        label={t("a11y.statusRegionLabel")}
        data-testid="admin-managers-live-status"
      />

      {state === "loading" && (
        <p className="text-sm text-slate-400" role="status">
          {t("admin.managers.loading")}
        </p>
      )}

      {(state === "forbidden" ||
        state === "unauthenticated" ||
        state === "service-error") && (
        <div className="rounded-lg border border-red-800 bg-red-950/50 p-4" role="alert">
          <p className="text-sm font-medium text-red-300">{statusMessage}</p>
          {state === "service-error" && (
            <button
              type="button"
              onClick={() => void load()}
              className="mt-3 rounded border border-red-700 px-3 py-1 text-sm text-red-200"
            >
              {t("admin.managers.retry")}
            </button>
          )}
        </div>
      )}

      {state === "empty" && (
        <p className="text-sm text-slate-400">{t("admin.managers.emptyTitle", { filter: "" })}</p>
      )}

      {state === "populated" && (
        <div className="overflow-x-auto rounded-lg border border-slate-800">
          <table className="min-w-full text-left text-sm">
            <thead className="bg-slate-900 text-slate-300">
              <tr>
                <th scope="col" className="px-3 py-2">{t("admin.managers.colName")}</th>
                <th scope="col" className="px-3 py-2">{t("admin.managers.colAddress")}</th>
                <th scope="col" className="px-3 py-2">{t("admin.managers.colStatus")}</th>
              </tr>
            </thead>
            <tbody>
              {managers.map((manager) => (
                <tr key={manager.id} className="border-t border-slate-800 text-slate-300">
                  <td className="px-3 py-2">{manager.name}</td>
                  <td className="px-3 py-2 font-mono text-xs">{manager.stellar_address}</td>
                  <td className="px-3 py-2">{manager.status}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
