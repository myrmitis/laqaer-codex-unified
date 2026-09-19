export type BrowserSessionState =
  | "logged_out"
  | "authenticating"
  | "healthy"
  | "challenged"
  | "needs_reauth";

export type WebMode =
  | "instant"
  | "medium"
  | "high"
  | "extra_high"
  | "pro";

export interface BrowserTurnIdentity {
  threadId?: string;
  turnId: string;
  requestKind?: string;
}

export interface BrowserTurnRequest {
  protocol: 1;
  traceId: string;
  identity: BrowserTurnIdentity;
  mode: WebMode;
  prompt: string;
  previousResponseId?: string;
}

export type BrowserFailureCode =
  | "web_auth_required"
  | "web_security_challenge"
  | "web_model_unavailable"
  | "web_dom_contract_changed"
  | "web_continuation_missing"
  | "web_transport_failed";

export type BrowserTurnResult =
  | {
      ok: true;
      responseId: string;
      text: string;
    }
  | {
      ok: false;
      code: BrowserFailureCode;
      message: string;
      retryable: boolean;
    };
