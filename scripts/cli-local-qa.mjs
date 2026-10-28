#!/usr/bin/env node
// Deterministic production CLI -> IPC host -> Core -> encrypted local room.
// All process payloads stay in memory; only fixed tokens/errors reach stdout.
import { spawn, spawnSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createInterface } from "node:readline";
import { freePort, minimalEnvironment, registerUser, startHomeserver, stopProcess, tuwunelConfig, waitForHomeserver } from "./lib/local-homeserver-qa.mjs";
import { writeSensitivePayloadToPath } from "./lib/sensitive-fifo.mjs";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const profile = process.argv.includes("--ci") ? "ci" : "debug";
if (process.argv.slice(2).some(arg => arg !== "--ci"))
    throw new Error("cli_qa_usage");
const target = process.env.CARGO_TARGET_DIR ? resolve(process.env.CARGO_TARGET_DIR) : join(root, "target");
const cli = join(target, profile, "koushi");
const host = join(target, profile, "headless-core-qa");
const run = mkdtempSync(join(tmpdir(), "koushi-cli-"));
const bridge = join(run, "bridge");
let server = null, app = null;
let stopping = false;
const clients = new Set();
async function cleanup() {
    if (stopping)
        return;
    stopping = true;
    await Promise.all([...clients].map(child => stopProcess(child)));
    await stopProcess(app);
    await stopProcess(server);
    rmSync(run, { recursive: true, force: true });
}
for (const signal of ["SIGINT", "SIGTERM"])
    process.once(signal, () => { cleanup().finally(() => process.exit(130)); });
function check(condition, code) { if (!condition)
    throw new Error(code); }
function ready(child) {
    return new Promise((resolvePromise, reject) => {
        const lines = createInterface({ input: child.stdout });
        const timer = setTimeout(() => { lines.close(); reject(new Error("cli_qa_host_timeout")); }, 180000);
        lines.on("line", line => {
            if (!line.startsWith("MCP_HOST_READY "))
                return;
            clearTimeout(timer);
            lines.close();
            try {
                resolvePromise(JSON.parse(line.slice(15)));
            }
            catch {
                reject(new Error("cli_qa_host_response"));
            }
        });
        child.once("error", () => { clearTimeout(timer); reject(new Error("cli_qa_host_spawn")); });
        child.once("exit", () => { clearTimeout(timer); reject(new Error("cli_qa_host_exit")); });
    });
}
function call(args, input = "") {
    return new Promise((resolvePromise, reject) => {
        const child = spawn(cli, [...args, "--json", "--timeout", "20"], { cwd: root, env: minimalEnvironment(), stdio: ["pipe", "pipe", "pipe"], detached: true });
        clients.add(child);
        let output = "", length = 0;
        const timer = setTimeout(() => { child.kill("SIGKILL"); reject(new Error("cli_qa_call_timeout")); }, 25000);
        child.stdout.on("data", chunk => { length += chunk.length; if (length > 16 * 1024 * 1024) {
            child.kill("SIGKILL");
            reject(new Error("cli_qa_output_bound"));
        }
        else
            output += chunk; });
        child.stderr.resume();
        child.once("error", () => { clearTimeout(timer); reject(new Error("cli_qa_cli_spawn")); });
        child.once("exit", code => { clients.delete(child); clearTimeout(timer); if (code !== 0) {
            reject(new Error("cli_qa_cli_failure"));
            return;
        } try {
            resolvePromise(JSON.parse(output));
        }
        catch {
            reject(new Error("cli_qa_cli_json"));
        } });
        child.stdin.on("error", () => { });
        child.stdin.end(input);
    });
}
try {
    const port = await freePort();
    const homeserver = `http://127.0.0.1:${port}`;
    const serverName = "cli-qa.localhost";
    const data = join(run, "server");
    mkdirSync(data);
    const config = join(run, "server.toml");
    const log = join(run, "server.log");
    writeFileSync(config, tuwunelConfig({ serverName, port, dataDir: data }));
    server = startHomeserver("tuwunel", config, log);
    await waitForHomeserver(homeserver, server, 60000, log);
    const user = `cli${randomBytes(4).toString("hex")}`;
    const password = randomBytes(24).toString("hex");
    await registerUser(homeserver, user, password);
    const login = join(run, "login.pipe");
    check(spawnSync("mkfifo", ["-m", "600", login], { env: minimalEnvironment(), stdio: "ignore" }).status === 0, "cli_qa_login_pipe");
    app = spawn(host, [], { cwd: root, env: { ...minimalEnvironment(), KOUSHI_LOCAL_QA_HOMESERVER: homeserver, KOUSHI_LOCAL_QA_SERVER_NAME: serverName, KOUSHI_QA_LOGIN_PIPE: login, KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR: join(run, "credentials"), KOUSHI_QA_DATA_DIR: join(run, "data"), KOUSHI_QA_MCP_HOST: "1", KOUSHI_QA_MCP_BRIDGE_DIR: bridge, KOUSHI_QA_MCP_CLIENT: "koushi-cli", KOUSHI_QA_CLI_FIXTURE: "1" }, stdio: ["ignore", "pipe", "pipe"], detached: true });
    app.stderr.resume();
    await writeSensitivePayloadToPath(login, JSON.stringify({ username: user, password }), 10000);
    const info = await ready(app);
    const endpoint = ["--endpoint", info.endpoint];
    const accounts = await call(["accounts", ...endpoint]);
    check(accounts.result.accounts.some(account => account.account_id === info.account_id), "cli_qa_accounts");
    const account = ["--account", info.account_id];
    const rooms = await call(["rooms", ...account, ...endpoint]);
    const room = rooms.result.rooms.find(room => room.name === "Synthetic CLI QA");
    check(room?.is_encrypted === true, "cli_qa_encrypted_room");
    const messages = await call(["messages", ...account, "--room", room.room_id, ...endpoint]);
    const message = messages.result.messages.find(message => message.body === "SYNTHETIC_CLI_SEARCH_PROOF");
    check(message?.event_id, "cli_qa_read");
    const single = await call(["message", ...account, "--room", room.room_id, "--event", message.event_id, ...endpoint]);
    check(single.result.body === "SYNTHETIC_CLI_SEARCH_PROOF" && single.result.is_encrypted, "cli_qa_message");
    // Indexing is asynchronous. Poll bounded reads until the fixture is indexed;
    // this is readiness observation, not rerunning a failed test/workflow.
    const deadline = Date.now() + 30000;
    let found = false;
    while (Date.now() < deadline) {
        const search = await call(["search", ...account, "--room", room.room_id, "--query-stdin", ...endpoint], "SYNTHETIC_CLI_SEARCH_PROOF");
        found = search.result.results?.some(result => result.event_id === message.event_id);
        if (found)
            break;
        await new Promise(resolvePromise => setTimeout(resolvePromise, 200));
    }
    check(found, "cli_qa_search");
    console.log("cli_accounts=ok\ncli_encrypted_read=ok\ncli_encrypted_search=ok");
}
catch (error) {
    // Never echo process payloads, identifiers, credentials, or SDK errors.
    console.error(/^cli_qa_[a-z_]+$/.test(error.message) ? error.message : "cli_qa_failed");
    process.exitCode = 1;
}
finally {
    await cleanup();
}
