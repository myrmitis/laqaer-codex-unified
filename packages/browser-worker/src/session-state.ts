import type {
  BrowserFailure,
  BrowserSessionState,
} from "./protocol.js";

const AUTH_PROVIDER_HOSTS = new Set([
  "auth.openai.com",
  "auth0.openai.com",
  "login.openai.com",
  "accounts.openai.com",
  "accounts.google.com",
  "login.microsoftonline.com",
  "appleid.apple.com",
  "idmsa.apple.com",
]);

export class BrowserSession {
  #state: BrowserSessionState = "logged_out";
  #revision = 0;

  get state(): BrowserSessionState {
    return this.#state;
  }

  get revision(): number {
    return this.#revision;
  }

  beginAuthentication(): void {
    this.#transition("authenticating");
  }

  markHealthy(): void {
    this.#transition("healthy");
  }

  markLoggedOut(): void {
    this.#transition("logged_out");
  }

  markChallenge(): void {
    this.#transition("challenged");
  }

  markNeedsReauth(): void {
    this.#transition("needs_reauth");
  }

  observeNavigation(url: string): BrowserFailure | undefined {
    if (!isAuthenticationUrl(url)) return undefined;
    this.markNeedsReauth();
    return authRequiredFailure(
      "ChatGPT redirected this browser session to authentication",
    );
  }

  canStartTurn(): boolean {
    return this.#state === "healthy";
  }

  preflightFailure(): BrowserFailure | undefined {
    switch (this.#state) {
      case "healthy":
        return undefined;
      case "challenged":
        return {
          ok: false,
          code: "web_security_challenge",
          message: "ChatGPT browser session is blocked by a security challenge",
          retryable: false,
        };
      case "logged_out":
      case "authenticating":
      case "needs_reauth":
        return authRequiredFailure(
          "ChatGPT browser session requires explicit sign-in",
        );
    }
  }

  #transition(next: BrowserSessionState): void {
    if (this.#state === next) return;
    this.#state = next;
    this.#revision += 1;
  }
}

export function isAuthenticationUrl(value: string): boolean {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return false;
  }

  if (url.protocol !== "https:") return false;

  if (url.hostname === "chatgpt.com") {
    return (
      url.pathname === "/auth" ||
      url.pathname.startsWith("/auth/") ||
      url.pathname === "/login"
    );
  }

  return AUTH_PROVIDER_HOSTS.has(url.hostname);
}

function authRequiredFailure(message: string): BrowserFailure {
  return {
    ok: false,
    code: "web_auth_required",
    message,
    retryable: false,
  };
}
