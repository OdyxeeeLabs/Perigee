"use client";

import { useRouter } from "next/router";
import { useState, useEffect } from "react";
import { useTranslations } from "next-intl";
import { ConnectButton } from "../../components/ConnectButton";
import { SEO } from "../../components/SEO";
import { Button } from "../../components/ui/Button";
import { useWalletStore } from "../../context/WalletContext";
import { managerService } from "../../lib/api";
import { shallow } from "../../lib/createStore";

type Step = "connect" | "register" | "submitted" | "status";

export default function ManagerOnboarding() {
  const t = useTranslations();
  const router = useRouter();
  const { address } = useWalletStore((s) => ({ address: s.address }), shallow);
  const [step, setStep] = useState<Step>("connect");
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [kycRef, setKycRef] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [managerRecord, setManagerRecord] = useState<Awaited<
    ReturnType<typeof managerService.register>
  > | null>(null);

  useEffect(() => {
    if (typeof window !== "undefined") {
      try {
        const savedDraft = sessionStorage.getItem("onboarding_draft");
        if (savedDraft) {
          const parsed = JSON.parse(savedDraft);
          if (parsed.name) setName(parsed.name);
          if (parsed.email) setEmail(parsed.email);
          if (parsed.kycRef) setKycRef(parsed.kycRef);
        }
      } catch {
        // ignore storage parse errors
      }
    }
  }, []);

  useEffect(() => {
    if (typeof window !== "undefined" && step === "register") {
      try {
        sessionStorage.setItem(
          "onboarding_draft",
          JSON.stringify({ name, email, kycRef }),
        );
      } catch {
        // ignore storage errors
      }
    }
  }, [name, email, kycRef, step]);

  useEffect(() => {
    if (address) {
      checkExisting(address);
    }
  }, [address]);

  async function checkExisting(addr: string) {
    try {
      const status = await managerService.checkStatus(addr);
      if (status.status === "approved") {
        setManagerRecord({
          id: status.id,
          stellar_address: addr,
          name: "",
          email: "",
          status: status.status,
          kyc_document_ref: "",
          notes: "",
          created_at: "",
          updated_at: "",
        });
        setStep("status");
      } else if (status.status === "pending") {
        setStep("submitted");
      } else if (status.status === "rejected") {
        setError(t("onboarding.rejectedMessage"));
        setStep("status");
      } else {
        setStep("register");
      }
    } catch {
      setStep("register");
    }
  }

  async function handleRegister(e: React.FormEvent) {
    e.preventDefault();
    if (!address) return;
    if (!name.trim()) {
      setError(t("onboarding.nameRequired"));
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const record = await managerService.register({
        stellar_address: address,
        name: name.trim(),
        email: email.trim(),
        kyc_document_ref: kycRef.trim(),
      });
      setManagerRecord(record);
      setStep("submitted");
      if (typeof window !== "undefined") {
        sessionStorage.removeItem("onboarding_draft");
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : t("onboarding.registrationFailed");
      setError(msg);
    } finally {
      setLoading(false);
    }
  }

  return (
    <>
      <SEO
        title={t("onboarding.title")}
        description={t("onboarding.description")}
        path="/managers/onboarding"
      />
      <main className="min-h-screen bg-slate-950 text-slate-100">
        <header className="sticky top-0 z-50 border-b border-slate-800 bg-slate-950/90 backdrop-blur">
          <div className="mx-auto flex max-w-6xl items-center justify-between px-4 py-4 sm:px-6 lg:px-8">
            <div>
              <h1 className="text-2xl font-bold text-cyan-400">{t("app.name")}</h1>
              <p className="text-sm text-slate-400">{t("onboarding.subtitle")}</p>
            </div>
            <ConnectButton />
          </div>
        </header>

        <section className="mx-auto max-w-2xl px-4 py-12 sm:px-6 lg:px-8">
          <div className="rounded-2xl border border-slate-800 bg-slate-900/70 p-8">
            <h2 className="mb-6 text-2xl font-semibold text-cyan-300">
              {step === "connect" && t("onboarding.stepConnect")}
              {step === "register" && t("onboarding.stepRegister")}
              {step === "submitted" && t("onboarding.stepSubmitted")}
              {step === "status" && t("onboarding.stepStatus")}
            </h2>

            {step === "connect" && (
              <div className="space-y-4">
                <p className="text-slate-400">
                  {t("onboarding.connectPrompt")}
                </p>
              </div>
            )}

            {step === "register" && (
              <form onSubmit={handleRegister} className="space-y-5">
                <div>
                  <label className="mb-1 block text-sm font-medium text-slate-300">
                    {t("onboarding.stellarAddress")}
                  </label>
                  <input
                    value={address || ""}
                    disabled
                    className="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 font-mono text-sm text-slate-400"
                  />
                </div>
                <div>
                  <label className="mb-1 block text-sm font-medium text-slate-300">
                    {t("onboarding.fullName")}
                  </label>
                  <input
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder={t("onboarding.fullNamePlaceholder")}
                    className="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-100 placeholder-slate-500"
                    required
                  />
                </div>
                <div>
                  <label className="mb-1 block text-sm font-medium text-slate-300">
                    {t("onboarding.email")}
                  </label>
                  <input
                    type="email"
                    value={email}
                    onChange={(e) => setEmail(e.target.value)}
                    placeholder={t("onboarding.emailPlaceholder")}
                    className="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-100 placeholder-slate-500"
                  />
                </div>
                <div>
                  <label className="mb-1 block text-sm font-medium text-slate-300">
                    {t("onboarding.kycRef")}
                  </label>
                  <input
                    value={kycRef}
                    onChange={(e) => setKycRef(e.target.value)}
                    placeholder={t("onboarding.kycRefPlaceholder")}
                    className="w-full rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-sm text-slate-100 placeholder-slate-500"
                  />
                </div>
                {error && (
                  <div className="rounded-lg border border-red-800 bg-red-950/50 px-4 py-3 text-sm text-red-400">
                    {error}
                  </div>
                )}
                <Button type="submit" disabled={loading}>
                  {loading ? t("onboarding.submittingButton") : t("onboarding.submitButton")}
                </Button>
              </form>
            )}

            {step === "submitted" && (
              <div className="space-y-4">
                <div className="rounded-lg border border-yellow-800 bg-yellow-950/50 px-4 py-3 text-sm text-yellow-400">
                  {t("onboarding.submittedMessage")}
                </div>
                {managerRecord && (
                  <div className="space-y-2 text-sm text-slate-400">
                    <p>
                      <span className="text-slate-300">{t("onboarding.id")}</span>{" "}
                      {managerRecord.id}
                    </p>
                    <p>
                      <span className="text-slate-300">{t("onboarding.status")}</span>{" "}
                      {managerRecord.status}
                    </p>
                  </div>
                )}
              </div>
            )}

            {step === "status" && (
              <div className="space-y-4">
                {error ? (
                  <div className="rounded-lg border border-red-800 bg-red-950/50 px-4 py-3 text-sm text-red-400">
                    {error}
                  </div>
                ) : (
                  <div className="rounded-lg border border-green-800 bg-green-950/50 px-4 py-3 text-sm text-green-400">
                    {t("onboarding.approvedMessage")}
                  </div>
                )}
              </div>
            )}

            <div className="mt-8 border-t border-slate-800 pt-4">
              <Button variant="link" onClick={() => router.push("/")}>
                {t("nav.backToAnalyzer")}
              </Button>
            </div>
          </div>
        </section>
      </main>
    </>
  );
}
