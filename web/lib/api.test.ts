import assert from "node:assert/strict";

import { validateAnalyzeRequest, validateAnalyzeWasmRequest } from "./api";

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
})();
