import { BrowserSession } from "./session-state.ts";
import type { BrowserTurnRequest, BrowserTurnResult } from "./protocol.ts";

const session = new BrowserSession();

export function sessionState() {
  return session.state;
}

export function browserSession(): BrowserSession {
  return session;
}

export async function executeBrowserTurnForSession(
  targetSession: BrowserSession,
  request: BrowserTurnRequest,
): Promise<BrowserTurnResult> {
  const preflight = targetSession.preflightFailure();
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

export async function executeBrowserTurn(
  request: BrowserTurnRequest,
): Promise<BrowserTurnResult> {
  return executeBrowserTurnForSession(session, request);
}
