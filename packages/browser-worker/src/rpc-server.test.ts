import assert from "node:assert/strict";
import test from "node:test";
import { BrowserSession } from "./session-state.ts";
import { startBrowserRpcServer } from "./rpc-server.ts";
import {
  BROWSER_RPC_PROTOCOL,
  type BrowserTurnRequest,
} from "./protocol.ts";

const CAPABILITY = "browser-test-capability-0123456789";
const HEADER = "x-codex-unified-browser-capability";

function requestBody(
  overrides: Partial<BrowserTurnRequest> = {},
): BrowserTurnRequest {
  return {
    protocol: BROWSER_RPC_PROTOCOL,
    traceId: "trace-1",
    identity: { turnId: "turn-1", threadId: "thread-1" },
    mode: "instant",
    prompt: "test",
    ...overrides,
  };
}

test("health is available without revealing session state", async t => {
  const session = new BrowserSession();
  const server = await startBrowserRpcServer({
    capability: CAPABILITY,
    session,
    executeTurn: async () => ({
      ok: true,
      responseId: "response-1",
      text: "ok",
    }),
  });
  t.after(() => server.close());

  const response = await fetch(`${server.origin}/healthz`);
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), {
    service: "codex-unified-browser-worker",
    protocol: 1,
    status: "ok",
  });
});

test("session state requires the local capability", async t => {
  const session = new BrowserSession();
  const server = await startBrowserRpcServer({
    capability: CAPABILITY,
    session,
    executeTurn: async () => ({
      ok: true,
      responseId: "response-1",
      text: "ok",
    }),
  });
  t.after(() => server.close());

  const denied = await fetch(`${server.origin}/v1/session`);
  assert.equal(denied.status, 401);

  const allowed = await fetch(`${server.origin}/v1/session`, {
    headers: { [HEADER]: CAPABILITY },
  });
  assert.equal(allowed.status, 200);
  assert.deepEqual(await allowed.json(), {
    protocol: 1,
    state: "logged_out",
    revision: 0,
  });
});

test("turn execution uses the injected healthy browser session", async t => {
  const session = new BrowserSession();
  session.beginAuthentication();
  session.markHealthy();
  let calls = 0;

  const server = await startBrowserRpcServer({
    capability: CAPABILITY,
    session,
    executeTurn: async request => {
      calls += 1;
      assert.equal(request.mode, "pro");
      return {
        ok: true,
        responseId: "response-pro",
        text: "PRO OK",
      };
    },
  });
  t.after(() => server.close());

  const response = await fetch(`${server.origin}/v1/turn`, {
    method: "POST",
    headers: {
      [HEADER]: CAPABILITY,
      "content-type": "application/json",
    },
    body: JSON.stringify(requestBody({ mode: "pro" })),
  });

  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), {
    ok: true,
    responseId: "response-pro",
    text: "PRO OK",
  });
  assert.equal(calls, 1);
});

test("protocol mismatch fails before browser execution", async t => {
  const session = new BrowserSession();
  session.beginAuthentication();
  session.markHealthy();
  let calls = 0;

  const server = await startBrowserRpcServer({
    capability: CAPABILITY,
    session,
    executeTurn: async () => {
      calls += 1;
      return {
        ok: true,
        responseId: "unexpected",
        text: "unexpected",
      };
    },
  });
  t.after(() => server.close());

  const body = {
    ...requestBody(),
    protocol: 2,
  };
  const response = await fetch(`${server.origin}/v1/turn`, {
    method: "POST",
    headers: {
      [HEADER]: CAPABILITY,
      "content-type": "application/json",
    },
    body: JSON.stringify(body),
  });

  assert.equal(response.status, 409);
  const failure = await response.json() as { code: string };
  assert.equal(failure.code, "rpc_protocol_mismatch");
  assert.equal(calls, 0);
});

test("oversized JSON fails before browser execution", async t => {
  const session = new BrowserSession();
  session.beginAuthentication();
  session.markHealthy();
  let calls = 0;

  const server = await startBrowserRpcServer({
    capability: CAPABILITY,
    session,
    executeTurn: async () => {
      calls += 1;
      return {
        ok: true,
        responseId: "unexpected",
        text: "unexpected",
      };
    },
  });
  t.after(() => server.close());

  const response = await fetch(`${server.origin}/v1/turn`, {
    method: "POST",
    headers: {
      [HEADER]: CAPABILITY,
      "content-type": "application/json",
    },
    body: JSON.stringify(requestBody({
      prompt: "x".repeat(2 * 1024 * 1024 + 1),
    })),
  });

  assert.equal(response.status, 413);
  assert.equal(calls, 0);
});
