import { BrowserSession } from "../src/session-state.ts";
import {
  startBrowserRpcServer,
} from "../src/rpc-server.ts";
import type {
  BrowserTurnRequest,
  BrowserTurnResult,
} from "../src/protocol.ts";

const capability = process.env.CODEX_UNIFIED_BROWSER_CAPABILITY;
if (!capability) throw new Error("CODEX_UNIFIED_BROWSER_CAPABILITY is required");

const port = Number.parseInt(
  process.env.CODEX_UNIFIED_BROWSER_PORT ?? "17991",
  10,
);
if (!Number.isInteger(port) || port <= 0 || port > 65535) {
  throw new Error("CODEX_UNIFIED_BROWSER_PORT must be a valid non-zero TCP port");
}

const session = new BrowserSession();
session.beginAuthentication();
session.markHealthy();

const executeTurn = async (
  request: BrowserTurnRequest,
): Promise<BrowserTurnResult> => {
  if (request.mode !== "pro") {
    return {
      ok: false,
      code: "web_model_unavailable",
      message: "contract fixture only accepts Pro",
      retryable: false,
    };
  }
  if (request.identity.turnId !== "turn-contract") {
    return {
      ok: false,
      code: "web_transport_failed",
      message: "unexpected turn identity",
      retryable: false,
    };
  }
  return {
    ok: true,
    responseId: "response-contract",
    text: "BROWSER RPC CONTRACT OK",
  };
};

const server = await startBrowserRpcServer({
  capability,
  session,
  executeTurn,
}, port);

process.stdout.write(JSON.stringify({
  ready: true,
  origin: server.origin,
}) + "\n");
