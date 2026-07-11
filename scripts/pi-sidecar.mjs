#!/usr/bin/env node
import readline from "node:readline";
import { pathToFileURL } from "node:url";
import { createRequire } from "node:module";
import { readFile } from "node:fs/promises";
import path from "node:path";

const PROTOCOL = 1;
const write = (value) => process.stdout.write(`${JSON.stringify(value)}\n`);
const fail = (id, code, message) => write({ v: PROTOCOL, id, type: "error", error: { code, message } });

function packageRoot() {
  if (process.env.ASTER_PI_NODE_MODULES) return process.env.ASTER_PI_NODE_MODULES;
  const candidates = [process.cwd(), new URL(".", import.meta.url).pathname];
  for (const base of candidates) {
    try { return path.dirname(path.dirname(path.dirname(createRequire(`${base}/package.json`).resolve("@mariozechner/pi-agent-core/package.json")))); } catch {}
  }
  return null;
}

async function loadPi() {
  const manifest = packageRoot();
  if (!manifest) throw new Error("set ASTER_PI_NODE_MODULES to a directory containing @mariozechner/pi-agent-core 0.73.0");
  const require = createRequire(path.join(manifest, "package.json"));
  const coreManifest = JSON.parse(await readFile(path.join(manifest, "@mariozechner/pi-agent-core/package.json"), "utf8"));
  const aiManifest = JSON.parse(await readFile(path.join(manifest, "@mariozechner/pi-ai/package.json"), "utf8"));
  if (coreManifest.version !== "0.73.0" || aiManifest.version !== "0.73.0") throw new Error(`unsupported Pi versions core=${coreManifest.version} ai=${aiManifest.version}`);
  const core = await import(pathToFileURL(path.join(manifest, "@mariozechner/pi-agent-core/dist/index.js")).href);
  const ai = await import(pathToFileURL(path.join(manifest, "@mariozechner/pi-ai/dist/index.js")).href);
  return { core, ai, versions: { agentCore: coreManifest.version, ai: aiManifest.version } };
}

const active = new Map();
async function run(req) {
  if (req.mode === "fixture") {
    write({ v: 1, id: req.id, type: "agent_start" });
    write({ v: 1, id: req.id, type: "message_delta", role: "assistant", text: `fixture:${req.input.prompt}` });
    if (req.input.fixtureTool) write({ v: 1, id: req.id, type: "tool_preflight", callId: "fixture-tool-1", name: req.input.fixtureTool.name, arguments: req.input.fixtureTool.arguments ?? {}, capability: req.input.fixtureTool.capability });
    write({ v: 1, id: req.id, type: "usage", inputTokens: 3, outputTokens: 5, totalTokens: 8 });
    write({ v: 1, id: req.id, type: "agent_end", stopReason: "stop" });
    return;
  }
  const { core, ai } = await loadPi();
  const [provider, modelId] = req.input.model.split("/", 2);
  if (!provider || !modelId) throw new Error("model must be provider/model-id");
  const model = ai.getModel(provider, modelId);
  const agent = new core.Agent({ initialState: { model, thinkingLevel: req.input.effort ?? "medium", systemPrompt: req.input.context ?? "", tools: [] } });
  active.set(req.id, agent);
  const unsubscribe = agent.subscribe((event) => {
    if (event.type === "agent_start") write({ v: 1, id: req.id, type: "agent_start" });
    else if (event.type === "agent_end") write({ v: 1, id: req.id, type: "agent_end", stopReason: "stop" });
    else if (event.type === "message_update" && event.assistantMessageEvent?.type === "text_delta") write({ v: 1, id: req.id, type: "message_delta", role: "assistant", text: event.assistantMessageEvent.delta });
    else if (event.type === "message_end" && event.message?.usage) { const u=event.message.usage; write({ v:1,id:req.id,type:"usage",inputTokens:u.input,outputTokens:u.output,totalTokens:u.totalTokens ?? (u.input+u.output) }); }
  });
  try { await agent.prompt(req.input.prompt); } finally { unsubscribe(); active.delete(req.id); }
}

const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
rl.on("line", async (line) => {
  let req; try { req = JSON.parse(line); } catch { return fail(null, "invalid_json", "request is not valid JSON"); }
  if (req.v !== PROTOCOL || typeof req.id !== "string") return fail(req.id ?? null, "invalid_request", "protocol v1 and string id are required");
  if (req.type === "discover") { try { const pi=await loadPi(); write({v:1,id:req.id,type:"ready",protocol:1,node:process.versions.node,versions:pi.versions,capabilities:["abort","context","effort","fixture","model","tool_preflight"]}); } catch(e) { fail(req.id,"startup",e.message); } return; }
  if (req.type === "abort") { active.get(req.targetId)?.abort(); write({v:1,id:req.id,type:"aborted",targetId:req.targetId}); return; }
  if (req.type !== "run" || !req.input) return fail(req.id,"invalid_request","run input is required");
  try { await run(req); } catch(e) { fail(req.id, e.name === "AbortError" ? "aborted" : "runtime", e.message); }
});
