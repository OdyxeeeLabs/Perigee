import assert from "node:assert/strict";
import {
  validateAnalyzeRequest,
  validateAnalyzeWasmRequest,
  apiClient,
  ApiError,
} from "./api";

void (async () => {
  const validRequest = await validateAnalyzeRequest({
    contract_id: "CAEZJVJ4N7P7GRUVD5NG5LYYH23AQHJUKQEUHW54LR5PGQX3V7FXD7Q",
    function_name: "transfer",
    args: ["alice", "bob", "100"],
    protocol_version: 20,
    enable_experimental: true,
  });
  assert.equal(validRequest.function_name, "transfer");
  await assert.rejects(
    () =>
      validateAnalyzeRequest({
        contract_id: "",
        function_name: "",
        protocol_version: 0,
      }),
    /contract_id|function_name|protocol_version/,
  );
  const validWasmRequest = await validateAnalyzeWasmRequest({
    wasm_bytes: "abc123",
    function_name: "mint",
    args: ["1"],
  });
  assert.equal(validWasmRequest.function_name, "mint");
  await assert.rejects(
    () => validateAnalyzeWasmRequest({ wasm_bytes: "", function_name: "" }),
    /wasm_bytes|function_name/,
  );

  // Retry/backoff: transient failures eventually succeed
  {
    const originalFetch = globalThis.fetch;
    let callCount = 0;

    globalThis.fetch = (async () => {
      callCount++;
      if (callCount < 3) {
        return new Response(JSON.stringify({ message: "Service unavailable" }), {
          status: 503,
          headers: { "content-type": "application/json" },
        });
      }
      return new Response(JSON.stringify({ ok: true }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }) as typeof fetch;

    const result = await apiClient.get<{ ok: boolean }>("/health");
    assert.equal(callCount, 3);
    assert.equal(result.ok, true);

    globalThis.fetch = originalFetch;
  }

  // Retry/backoff: gives up after max attempts on persistent failure
  {
    const originalFetch = globalThis.fetch;
    let callCount = 0;

    globalThis.fetch = (async () => {
      callCount++;
      return new Response(JSON.stringify({ message: "Service unavailable" }), {
        status: 503,
        headers: { "content-type": "application/json" },
      });
    }) as typeof fetch;

    await assert.rejects(() => apiClient.get("/health"), ApiError);
    assert.equal(callCount, 4);

    globalThis.fetch = originalFetch;
  }
})();
