import { BrowserSession } from "./session-state.js";
import type { BrowserTurnRequest, BrowserTurnResult } from "./protocol.js";

const session = new BrowserSession();

export function sessionState() {
  return session.state;
}

export function browserSession(): BrowserSession {
  return session;
}

export async function executeBrowserTurn(
  request: BrowserTurnRequest,
): Promise<BrowserTurnResult> {
  const preflight = session.preflightFailure();
  if (preflight) return preflight;

  // Electron/Playwright execution lands in the next Phase 2 slice. This
  // placeholder is intentionally terminal and non-retryable rather than
  // pretending the browser model ran.
  return {
    ok: false,
    code: "web_transport_failed",
    message: `Browser transport is not implemented for ${request.mode}`,
    retryable: false,
  };
}
