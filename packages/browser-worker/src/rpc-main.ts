import {
  browserSession,
  executeBrowserTurn,
} from "./index.ts";
import {
  BROWSER_RPC_PROTOCOL,
} from "./protocol.ts";
import {
  startBrowserRpcServer,
} from "./rpc-server.ts";

const capability = process.env.CODEX_UNIFIED_BROWSER_CAPABILITY;
if (!capability) {
  throw new Error("CODEX_UNIFIED_BROWSER_CAPABILITY is required");
}

const rawPort = process.env.CODEX_UNIFIED_BROWSER_PORT ?? "0";
const port = Number.parseInt(rawPort, 10);
if (!Number.isInteger(port) || port < 0 || port > 65535) {
  throw new Error("CODEX_UNIFIED_BROWSER_PORT must be a valid TCP port");
}

const server = await startBrowserRpcServer({
  capability,
  session: browserSession(),
  executeTurn: executeBrowserTurn,
}, port);

process.stdout.write(JSON.stringify({
  service: "codex-unified-browser-worker",
  protocol: BROWSER_RPC_PROTOCOL,
  port: server.port,
}) + "\n");
