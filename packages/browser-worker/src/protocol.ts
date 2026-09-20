export const BROWSER_RPC_PROTOCOL = 1 as const;

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
  protocol: typeof BROWSER_RPC_PROTOCOL;
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

export interface BrowserFailure {
  ok: false;
  code: BrowserFailureCode;
  message: string;
  retryable: boolean;
}

export type BrowserTurnResult =
  | {
      ok: true;
      responseId: string;
      text: string;
    }
  | BrowserFailure;

export interface BrowserSessionSnapshot {
  protocol: typeof BROWSER_RPC_PROTOCOL;
  state: BrowserSessionState;
  revision: number;
}

export interface BrowserRpcHealth {
  service: "codex-unified-browser-worker";
  protocol: typeof BROWSER_RPC_PROTOCOL;
  status: "ok";
}

export type BrowserRpcFailureCode =
  | "rpc_unauthorized"
  | "rpc_protocol_mismatch"
  | "rpc_invalid_request"
  | "rpc_payload_too_large"
  | "rpc_internal_error";

export interface BrowserRpcFailure {
  ok: false;
  code: BrowserRpcFailureCode;
  message: string;
}
