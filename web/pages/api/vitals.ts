import type { NextApiRequest, NextApiResponse } from "next";
import { summarize } from "../../lib/vitalsStats";
import type { VitalPayload } from "../../lib/webVitals";

/**
 * Web vitals collector (issue #693).
 *
 *  - POST  stores a sample in a bounded in-memory buffer (and forwards it to
 *          `VITALS_FORWARD_URL` — e.g. a Plausible/Umami/custom endpoint — when
 *          configured). The buffer is per server instance, so for multi-instance
 *          deployments forward samples to durable analytics.
 *  - GET   returns p50/p75/p95 per metric, page, device and connection plus
 *          threshold alerts. Protected by `VITALS_ADMIN_TOKEN` (Bearer) when
 *          set; open only outside production otherwise.
 *
 * This file takes precedence over the `/api/[[...path]]` backend proxy.
 */

const MAX_SAMPLES = 5000;
const samples: VitalPayload[] = [];

const ALLOWED_DEVICES = new Set(["mobile", "tablet", "desktop"]);

function isValidSample(value: unknown): value is VitalPayload {
  if (!value || typeof value !== "object") return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.name === "string" &&
    v.name.length <= 16 &&
    typeof v.value === "number" &&
    Number.isFinite(v.value) &&
    typeof v.page === "string" &&
    v.page.length <= 200 &&
    typeof v.device === "string" &&
    ALLOWED_DEVICES.has(v.device) &&
    typeof v.connection === "string" &&
    v.connection.length <= 16
  );
}

function authorizedForRead(req: NextApiRequest): boolean {
  const token = process.env.VITALS_ADMIN_TOKEN;
  if (!token) return process.env.NODE_ENV !== "production";
  return req.headers.authorization === `Bearer ${token}`;
}

export default async function handler(req: NextApiRequest, res: NextApiResponse) {
  if (req.method === "POST") {
    const body = typeof req.body === "string" ? safeParse(req.body) : req.body;
    if (!isValidSample(body)) {
      return res.status(400).json({ error: "invalid vital sample" });
    }

    const sample: VitalPayload = {
      name: body.name,
      value: body.value,
      rating: body.rating,
      page: body.page,
      device: body.device,
      connection: body.connection,
      ts: Date.now(),
    };
    samples.push(sample);
    if (samples.length > MAX_SAMPLES) samples.splice(0, samples.length - MAX_SAMPLES);

    const forwardUrl = process.env.VITALS_FORWARD_URL;
    if (forwardUrl) {
      void fetch(forwardUrl, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(sample),
      }).catch(() => undefined);
    }
    return res.status(204).end();
  }

  if (req.method === "GET") {
    if (!authorizedForRead(req)) return res.status(401).json({ error: "unauthorized" });
    return res.status(200).json({ sampleCount: samples.length, ...summarize(samples) });
  }

  res.setHeader("Allow", "GET, POST");
  return res.status(405).json({ error: "method not allowed" });
}

function safeParse(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return undefined;
  }
}
