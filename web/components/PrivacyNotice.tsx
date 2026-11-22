"use client";

import { useEffect, useState } from "react";
import { useTranslations } from "next-intl";
import { privacyNotice } from "@/lib/config";
import {
  dismissPrivacyNotice,
  isPrivacyNoticeDismissed,
} from "@/lib/privacyNotice";

export function PrivacyNotice() {
  const t = useTranslations();
  const [dismissed, setDismissed] = useState(true);

  useEffect(() => {
    setDismissed(isPrivacyNoticeDismissed());
  }, []);

  if (!privacyNotice.enabled || dismissed) return null;

  return (
    <section
      aria-label={t("privacyNotice.ariaLabel")}
      className="mb-3 rounded-lg border border-slate-700 bg-slate-800/60 p-3 text-[11px] leading-relaxed text-slate-300"
    >
      <div className="mb-1 flex items-start justify-between gap-2">
        <h3 className="font-semibold text-slate-100">
          {t("privacyNotice.title")}
        </h3>
        <button
          type="button"
          onClick={() => {
            dismissPrivacyNotice();
            setDismissed(true);
          }}
          className="shrink-0 rounded text-slate-400 underline underline-offset-2 hover:text-slate-200 focus:outline-none focus:ring-2 focus:ring-cyan-500"
        >
          {t("privacyNotice.dismiss")}
        </button>
      </div>

      <p>{t("privacyNotice.intro")}</p>
      <ul className="mt-2 list-disc space-y-1 pl-4">
        <li>{t("privacyNotice.analytics")}</li>
        <li>{t("privacyNotice.errors")}</li>
        <li>{t("privacyNotice.wallet")}</li>
      </ul>

      {privacyNotice.legalNote ? (
        <p className="mt-2 border-l-2 border-slate-600 pl-2">
          <span className="font-medium">{t("privacyNotice.legalNoteLabel")}: </span>
          {privacyNotice.legalNote}
        </p>
      ) : null}

      <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1">
        <a
          href={privacyNotice.privacyPolicyUrl}
          target="_blank"
          rel="noopener noreferrer"
          className="text-cyan-400 underline underline-offset-2 hover:text-cyan-300"
        >
          {t("privacyNotice.privacyPolicy")}
        </a>
        <a
          href={privacyNotice.telemetryDocsUrl}
          target="_blank"
          rel="noopener noreferrer"
          className="text-cyan-400 underline underline-offset-2 hover:text-cyan-300"
        >
          {t("privacyNotice.details")}
        </a>
        <span className="text-slate-400">{t("privacyNotice.manage")}</span>
      </div>
    </section>
  );
}
