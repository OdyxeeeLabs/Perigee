"use client";

import type { InvocationResult } from "../lib/sorobantypes";
import { useState } from "react";
import { useTranslations } from "next-intl";
import { supportLinks, supportMailto } from "../lib/config";
import { CallGraphVisualizer } from "./CallGraphVisualizer";

interface ResultViewerProps {
  result: InvocationResult | null;
}

export function ResultViewer({ result }: ResultViewerProps) {
  const t = useTranslations();
  const [isExpanded, setIsExpanded] = useState(true);
  const [copied, setCopied] = useState(false);

  const downloadSnapshot = () => {
    if (!result?.stateSnapshot) return;
    const blob = new Blob([JSON.stringify(result.stateSnapshot, null, 2)], {
      type: "application/json",
    });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `Perigee-snapshot-${result.functionName}-${Date.now()}.json`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };

  const copyFullResult = () => {
    if (!result) return;
    navigator.clipboard.writeText(JSON.stringify(result, null, 2));
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  if (!result) {
    return (
      <div
        style={{
          padding: "24px",
          backgroundColor: "#0d1117",
          borderRadius: "8px",
          textAlign: "center",
          color: "#8b949e",
          border: "1px solid #30363d",
        }}
      >
        <p>{t("result.noResults")}</p>
      </div>
    );
  }

  return (
    <div
      className="print-container"
      style={{
        padding: "24px",
        backgroundColor: "#0d1117",
        borderRadius: "8px",
        borderLeft: `4px solid ${result.success ? "#00d9ff" : "#fb8500"}`,
        border: `1px solid #30363d`,
        maxHeight: "600px",
        overflowY: "auto",
        overflowX: "hidden",
      }}
    >
      <div
        style={{
          marginBottom: "16px",
          display: "flex",
          flexWrap: "wrap",
          gap: "12px",
          justifyContent: "space-between",
          alignItems: "center",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: "12px" }}>
          <button
            onClick={() => setIsExpanded(!isExpanded)}
            className="focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500"
            style={{
              background: "none",
              border: "none",
              color: "#8b949e",
              cursor: "pointer",
              fontSize: "14px",
              padding: "4px",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
            }}
            title={isExpanded ? t("result.collapse") : t("result.expand")}
          >
            {isExpanded ? "▼" : "▶"}
          </button>
          <div>
            <h3
              style={{
                margin: "0 0 4px 0",
                color: result.success ? "#00d9ff" : "#fb8500",
                fontSize: "16px",
                fontWeight: "600",
              }}
            >
              {result.success ? t("result.success") : t("result.error")}
            </h3>
            <p style={{ margin: "0", color: "#8b949e", fontSize: "12px" }}>
              {new Date(result.timestamp).toLocaleString()}
            </p>
          </div>
        </div>

        <div style={{ display: "flex", gap: "8px" }}>
          <button
            onClick={copyFullResult}
            className="focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500"
            style={{
              padding: "6px 12px",
              backgroundColor: "#1f2937",
              color: "#f3f4f6",
              borderRadius: "6px",
              border: "1px solid #374151",
              fontSize: "12px",
              cursor: "pointer",
              transition: "background-color 0.2s",
            }}
            onMouseOver={(e) =>
              (e.currentTarget.style.backgroundColor = "#374151")
            }
            onMouseOut={(e) =>
              (e.currentTarget.style.backgroundColor = "#1f2937")
            }
          >
            {copied ? t("result.copied") : t("result.copyFull")}
          </button>
          {result.stateSnapshot && (
            <button
              onClick={downloadSnapshot}
              className="focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500"
              style={{
                padding: "6px 12px",
                backgroundColor: "#1f2937",
                color: "#f3f4f6",
                borderRadius: "6px",
                border: "1px solid #374151",
                fontSize: "12px",
                cursor: "pointer",
                transition: "background-color 0.2s",
              }}
              onMouseOver={(e) =>
                (e.currentTarget.style.backgroundColor = "#374151")
              }
              onMouseOut={(e) =>
                (e.currentTarget.style.backgroundColor = "#1f2937")
              }
            >
              {t("result.downloadSnapshot")}
            </button>
          )}
        </div>
      </div>

      {isExpanded && (
        <>
          {result.error ? (
            <div
              style={{
                backgroundColor: "#0d1117",
                padding: "16px",
                borderRadius: "6px",
                marginBottom: "12px",
                fontSize: "13px",
                border: "1px solid #fb8500",
              }}
            >
              <div style={{ marginBottom: "12px" }}>
                <div
                  style={{
                    color: "#fb8500",
                    fontWeight: "600",
                    marginBottom: "8px",
                    display: "flex",
                    alignItems: "center",
                    gap: "8px",
                  }}
                >
                  {t("result.errorDetails")}
                  {result.errorType && (
                    <span
                      style={{
                        fontSize: "11px",
                        backgroundColor: "#2d1810",
                        color: "#f0883e",
                        padding: "2px 8px",
                        borderRadius: "3px",
                        border: "1px solid #fb8500",
                        fontFamily: "monospace",
                        fontWeight: "normal",
                      }}
                    >
                      {result.errorType}
                    </span>
                  )}
                </div>
                <div
                  style={{
                    backgroundColor: "#1a1f26",
                    padding: "12px",
                    borderRadius: "4px",
                    color: "#f0883e",
                    fontFamily: "monospace",
                    whiteSpace: "pre-wrap",
                    wordBreak: "break-word",
                    border: "1px solid #30363d",
                  }}
                >
                  {result.error}
                </div>
              </div>
              <div
                style={{ fontSize: "12px", color: "#8b949e", lineHeight: 1.6 }}
              >
                {result.errorType === "NETWORK_ERROR" ? (
                  <>
                    {t("result.networkError")}
                    <br />
                    Start it with{" "}
                    <code style={{ color: "#00d9ff" }}>cargo run</code>{" "}
                    (expected at{" "}
                    <code style={{ color: "#00d9ff" }}>localhost:8080</code>),
                    then retry.
                  </>
                ) : result.errorType === "PARSE_ERROR" ? (
                  <>{t("result.parseError")}</>
                ) : result.errorType === "INTERNAL_SERVER_ERROR" ? (
                  <>{t("result.internalError")}</>
                ) : (
                  <>{t("result.tip")}</>
                )}
                <br />
                <a
                  href={supportMailto(
                    `Perigee error report (${result.errorType ?? "UNKNOWN"})`,
                  )}
                  style={{ color: "#00d9ff", textDecoration: "underline" }}
                >
                  {t("result.contactSupport")}
                </a>
                {" · "}
                <a
                  href={supportLinks.docsUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                  style={{ color: "#00d9ff", textDecoration: "underline" }}
                >
                  {t("result.viewDocs")}
                </a>
              </div>
            </div>
          ) : (
            Boolean(result.result) && (
              <div
                style={{
                  backgroundColor: "#0d1117",
                  padding: "12px",
                  borderRadius: "6px",
                  marginBottom: "12px",
                  fontSize: "13px",
                  fontFamily: "monospace",
                  whiteSpace: "pre-wrap",
                  wordBreak: "break-all",
                  color: "#58a6ff",
                  border: "1px solid #30363d",
                }}
              >
                <strong style={{ color: "#8b949e" }}>{t("result.label")}</strong>
                <br />
                {JSON.stringify(result.result, null, 2)}
              </div>
            )
          )}

          {result.callGraphMermaid && (
            <CallGraphVisualizer mermaidDefinition={result.callGraphMermaid} />
          )}
        </>
      )}
    </div>
  );
}
