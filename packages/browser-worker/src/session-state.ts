import type { BrowserSessionState } from "./protocol.js";

export class BrowserSession {
  #state: BrowserSessionState = "logged_out";

  get state(): BrowserSessionState {
    return this.#state;
  }

  markAuthenticating(): void {
    this.#state = "authenticating";
  }

  markHealthy(): void {
    this.#state = "healthy";
  }

  markChallenge(): void {
    this.#state = "challenged";
  }

  markNeedsReauth(): void {
    this.#state = "needs_reauth";
  }

  canStartTurn(): boolean {
    return this.#state === "healthy";
  }
}
