import assert from "node:assert/strict";
import test from "node:test";
import {
  BrowserSession,
  isAuthenticationUrl,
} from "./session-state.ts";

test("logged-out and needs-reauth sessions fail before browser work", () => {
  const session = new BrowserSession();

  assert.equal(session.canStartTurn(), false);
  assert.equal(session.preflightFailure()?.code, "web_auth_required");

  session.beginAuthentication();
  assert.equal(session.preflightFailure()?.code, "web_auth_required");

  session.markHealthy();
  assert.equal(session.canStartTurn(), true);
  assert.equal(session.preflightFailure(), undefined);

  session.markNeedsReauth();
  assert.equal(session.canStartTurn(), false);
  assert.equal(session.preflightFailure()?.code, "web_auth_required");
});

test("security challenge is distinct from authentication failure", () => {
  const session = new BrowserSession();
  session.beginAuthentication();
  session.markHealthy();
  session.markChallenge();

  const failure = session.preflightFailure();
  assert.equal(session.canStartTurn(), false);
  assert.equal(failure?.code, "web_security_challenge");
  assert.equal(failure?.retryable, false);
});

test("authentication redirect during a healthy session fails closed", () => {
  const session = new BrowserSession();
  session.beginAuthentication();
  session.markHealthy();

  const revision = session.revision;
  const failure = session.observeNavigation("https://chatgpt.com/auth/login");

  assert.equal(failure?.code, "web_auth_required");
  assert.equal(session.state, "needs_reauth");
  assert.equal(session.revision, revision + 1);
  assert.equal(session.canStartTurn(), false);
});

test("identity-provider navigation is classified as authentication", () => {
  assert.equal(isAuthenticationUrl("https://auth.openai.com/login"), true);
  assert.equal(isAuthenticationUrl("https://accounts.google.com/o/oauth2/v2/auth"), true);
  assert.equal(isAuthenticationUrl("https://chatgpt.com/?temporary-chat=true"), false);
  assert.equal(isAuthenticationUrl("https://example.com/login"), false);
  assert.equal(isAuthenticationUrl("not a url"), false);
});

test("explicit authentication can restore healthy state", () => {
  const session = new BrowserSession();
  session.markNeedsReauth();
  session.beginAuthentication();
  session.markHealthy();

  assert.equal(session.state, "healthy");
  assert.equal(session.canStartTurn(), true);
  assert.equal(session.preflightFailure(), undefined);
});
