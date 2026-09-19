import { BrowserSession } from "./session-state.js";
import type { BrowserTurnRequest, BrowserTurnResult } from "./protocol.js";

const session = new BrowserSession();

export function sessionState() {
  return session.state;
}

export async function executeBrowserTurn(
  request: BrowserTurnRequest,
): Promise<BrowserTurnResult> {
  if (!session.canStartTurn()) {
    return {
      ok: false,
      code: "web_auth_required",
      message: "ChatGPT browser session must be healthy before a turn starts",
      retryable: false,
    };
  }

  // Electron/Playwright execution lands in Phase 2.
  return {
    ok: false,
    code: "web_transport_failed",
    message: `Browser transport is not implemented for ${request.mode}`,
    retryable: false,
  };
}
