import { Miniflare } from "miniflare";
import { describe, it, expect, beforeAll, afterAll } from "vitest";
import { resolve } from "path";

let mf: Miniflare;

beforeAll(async () => {
  const scriptPath = resolve(
    __dirname,
    "../../crates/edgeclaw-worker/build/worker/shim.mjs",
  );

  mf = new Miniflare({
    modules: true,
    scriptPath,
    modulesRules: [
      { type: "ESModule", include: ["**/*.js"] },
      { type: "CompiledWasm", include: ["**/*.wasm"] },
    ],
    durableObjects: {
      AGENT_DO: "AgentDo",
    },
    bindings: {
      CLAUDE_MODEL: "claude-sonnet-4-20250514",
    },
  });
});

afterAll(async () => {
  await mf?.dispose();
});

describe("Dispatcher", () => {
  it("returns 400 without user identity", async () => {
    const resp = await mf.dispatchFetch("http://localhost/message", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ message: "hello" }),
    });
    expect(resp.status).toBe(400);
    const text = await resp.text();
    expect(text).toContain("Missing user identity");
  });

  it("routes to DO with X-User-Id header", async () => {
    const resp = await mf.dispatchFetch("http://localhost/history", {
      method: "GET",
      headers: { "X-User-Id": "test-user-1" },
    });
    // Should get through to the DO (200 with empty history)
    expect(resp.status).toBe(200);
  });
});

describe("AgentDO", () => {
  it("GET /history returns empty array for new agent", async () => {
    const resp = await mf.dispatchFetch("http://localhost/history", {
      method: "GET",
      headers: { "X-User-Id": "fresh-user" },
    });
    expect(resp.status).toBe(200);
    const body = await resp.json();
    expect(body).toEqual([]);
  });

  it("GET / with Upgrade: websocket returns 101", async () => {
    const resp = await mf.dispatchFetch("http://localhost/", {
      method: "GET",
      headers: {
        "X-User-Id": "ws-user",
        Upgrade: "websocket",
      },
    });
    expect(resp.status).toBe(101);
    expect(resp.webSocket).toBeTruthy();
  });
});
