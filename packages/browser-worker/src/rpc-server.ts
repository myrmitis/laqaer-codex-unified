import { timingSafeEqual } from "node:crypto";
import {
  createServer,
  type IncomingMessage,
  type Server,
  type ServerResponse,
} from "node:http";
import type { AddressInfo } from "node:net";
import { BrowserSession } from "./session-state.ts";
import {
  BROWSER_RPC_PROTOCOL,
  type BrowserRpcFailure,
  type BrowserRpcHealth,
  type BrowserSessionSnapshot,
  type BrowserTurnRequest,
  type BrowserTurnResult,
  type WebMode,
} from "./protocol.ts";

const CAPABILITY_HEADER = "x-codex-unified-browser-capability";
const MAX_JSON_BYTES = 2 * 1024 * 1024;
const WEB_MODES = new Set<WebMode>([
  "instant",
  "medium",
  "high",
  "extra_high",
  "pro",
]);

export type BrowserTurnExecutor = (
  request: BrowserTurnRequest,
) => Promise<BrowserTurnResult>;

export interface BrowserRpcServerOptions {
  capability: string;
  session: BrowserSession;
  executeTurn: BrowserTurnExecutor;
}

export interface StartedBrowserRpcServer {
  origin: string;
  port: number;
  close(): Promise<void>;
}

export function createBrowserRpcServer(
  options: BrowserRpcServerOptions,
): Server {
  if (options.capability.length < 24) {
    throw new Error("Browser RPC capability must contain at least 24 characters");
  }

  return createServer(async (request, response) => {
    try {
      await handleRequest(options, request, response);
    } catch {
      writeJson(response, 500, rpcFailure(
        "rpc_internal_error",
        "Browser RPC request failed internally",
      ));
    }
  });
}

export async function startBrowserRpcServer(
  options: BrowserRpcServerOptions,
  port = 0,
): Promise<StartedBrowserRpcServer> {
  const server = createBrowserRpcServer(options);
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", () => {
      server.off("error", reject);
      resolve();
    });
  });

  const address = server.address();
  if (!address || typeof address === "string") {
    await closeServer(server);
    throw new Error("Browser RPC server did not expose a TCP address");
  }

  const portNumber = (address as AddressInfo).port;
  return {
    origin: `http://127.0.0.1:${portNumber}`,
    port: portNumber,
    close: () => closeServer(server),
  };
}

async function handleRequest(
  options: BrowserRpcServerOptions,
  request: IncomingMessage,
  response: ServerResponse,
): Promise<void> {
  const url = new URL(request.url ?? "/", "http://127.0.0.1");

  if (request.method === "GET" && url.pathname === "/healthz") {
    const health: BrowserRpcHealth = {
      service: "codex-unified-browser-worker",
      protocol: BROWSER_RPC_PROTOCOL,
      status: "ok",
    };
    writeJson(response, 200, health);
    return;
  }

  if (!authorized(request, options.capability)) {
    writeJson(response, 401, rpcFailure(
      "rpc_unauthorized",
      "Browser RPC capability is invalid",
    ));
    return;
  }

  if (request.method === "GET" && url.pathname === "/v1/session") {
    const snapshot: BrowserSessionSnapshot = {
      protocol: BROWSER_RPC_PROTOCOL,
      state: options.session.state,
      revision: options.session.revision,
    };
    writeJson(response, 200, snapshot);
    return;
  }

  if (request.method === "POST" && url.pathname === "/v1/turn") {
    let body: unknown;
    try {
      body = await readJson(request);
    } catch (error) {
      if (error instanceof PayloadTooLargeError) {
        writeJson(response, 413, rpcFailure(
          "rpc_payload_too_large",
          "Browser RPC JSON payload exceeds the configured limit",
        ));
        return;
      }
      writeJson(response, 400, rpcFailure(
        "rpc_invalid_request",
        "Browser RPC request body must be valid JSON",
      ));
      return;
    }

    if (!isBrowserTurnRequest(body)) {
      const protocol = readProtocol(body);
      if (protocol !== undefined && protocol !== BROWSER_RPC_PROTOCOL) {
        writeJson(response, 409, rpcFailure(
          "rpc_protocol_mismatch",
          `Browser RPC protocol ${protocol} is incompatible with protocol ${BROWSER_RPC_PROTOCOL}`,
        ));
        return;
      }
      writeJson(response, 400, rpcFailure(
        "rpc_invalid_request",
        "Browser RPC turn request is invalid",
      ));
      return;
    }

    const result = await options.executeTurn(body);
    writeJson(response, 200, result);
    return;
  }

  writeJson(response, 404, rpcFailure(
    "rpc_invalid_request",
    "Browser RPC endpoint was not found",
  ));
}

function authorized(
  request: IncomingMessage,
  expectedCapability: string,
): boolean {
  const received = request.headers[CAPABILITY_HEADER];
  if (typeof received !== "string") return false;

  const expectedBytes = Buffer.from(expectedCapability, "utf8");
  const receivedBytes = Buffer.from(received, "utf8");
  return expectedBytes.length === receivedBytes.length
    && timingSafeEqual(expectedBytes, receivedBytes);
}

async function readJson(request: IncomingMessage): Promise<unknown> {
  const chunks: Buffer[] = [];
  let bytes = 0;

  for await (const raw of request) {
    const chunk = Buffer.isBuffer(raw) ? raw : Buffer.from(raw);
    bytes += chunk.length;
    if (bytes > MAX_JSON_BYTES) throw new PayloadTooLargeError();
    chunks.push(chunk);
  }

  const body = Buffer.concat(chunks).toString("utf8");
  return JSON.parse(body);
}

function isBrowserTurnRequest(value: unknown): value is BrowserTurnRequest {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<BrowserTurnRequest>;
  return candidate.protocol === BROWSER_RPC_PROTOCOL
    && typeof candidate.traceId === "string"
    && candidate.traceId.length > 0
    && !!candidate.identity
    && typeof candidate.identity.turnId === "string"
    && candidate.identity.turnId.length > 0
    && typeof candidate.mode === "string"
    && WEB_MODES.has(candidate.mode as WebMode)
    && typeof candidate.prompt === "string";
}

function readProtocol(value: unknown): number | undefined {
  if (!value || typeof value !== "object") return undefined;
  const protocol = (value as { protocol?: unknown }).protocol;
  return typeof protocol === "number" ? protocol : undefined;
}

function writeJson(
  response: ServerResponse,
  status: number,
  body: unknown,
): void {
  response.statusCode = status;
  response.setHeader("content-type", "application/json; charset=utf-8");
  response.setHeader("cache-control", "no-store");
  response.end(JSON.stringify(body));
}

function rpcFailure(
  code: BrowserRpcFailure["code"],
  message: string,
): BrowserRpcFailure {
  return { ok: false, code, message };
}

function closeServer(server: Server): Promise<void> {
  return new Promise((resolve, reject) => {
    server.close(error => error ? reject(error) : resolve());
  });
}

class PayloadTooLargeError extends Error {}
