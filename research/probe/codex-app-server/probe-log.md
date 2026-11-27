# Codex app-server probe log

> Every claim the Phase 9 Codex source will make about the app-server, traced to the request that
> produced it. Rows are append-only; numbers are permanent.
>
> **Protocol:** newline-delimited JSON-RPC 2.0 over stdio (`codex app-server --stdio`). No socket
> mode in this build (`--help`: subcommands `daemon`, `proxy`, `generate-ts`,
> `generate-json-schema`; the only transport flag is `--stdio`). JSON Schema generated with
> `codex app-server generate-json-schema` → `schema/` (110 client request methods in
> `ClientRequest.json` — the live server answers 192, see [#20](#)).
>
> **Setup:** every run used a throwaway `CODEX_HOME` (`run*/codex-home`), logged out (no
> `auth.json`), and a local mock OpenAI-compatible provider (`mock-provider.py`, port 8765)
> injected via `-c model_providers.mock.{name,base_url,wire_api,env_key}`. Nothing ever touched
> the operator's plan or API key. Tooling: `probe2.py` (reader-thread JSON-RPC client with
> timestamped wire log), `client.py` (first sync client), `mock-provider.py`. Wire logs:
> `run*/wire.log`; server internals: `run*/codex-home/logs_2.sqlite`.
>
> Codex CLI 0.171.0. All rows below measured against that build.

## Transport

| # | Claim | How | Result |
|---|---|---|---|
| 1 | Wire is NDJSON JSON-RPC 2.0 over stdio | `probe2.py` against `codex app-server --stdio` | Confirmed. Requests `{"jsonrpc":"2.0","id":N,"method":M,"params":{...}}` one per line; responses and notifications same line format; no content-length framing |
| 2 | Handshake is `initialize` → result → `initialized` notification | sent both | `initialize` result: `{userAgent, codexHome, platformFamily, platformOs}` — **no capabilities field**. `initialized` (no id) acked silently. Server pushes a `remoteControl/status/changed` notification at startup (`status: "disabled"` logged out, with `serverName`, `installationId`) |
| 3 | **Responses are flushed one request behind** | run4: single request, then silence; run6: burst of 5 requests | The `initialize` response is written immediately; **every later response and notification is buffered and flushed only when the server reads the next stdin line**. Notifications do not flush (run4 keepalives = `initialized` notifications changed nothing). A request/response client must pipeline (keep a request in flight) or send cheap nudge requests (`account/read` answered in <1 ms), else it hangs. A naive "send, wait 15 s, give up" client (run3) saw every response arrive ~1 s after its own timeout |
| 4 | Handlers are fast; earlier "15–30 s stalls" were the flush rule, not the server | `logs_2.sqlite` timestamps vs wire log | Every handler (initialize, model/list, account/*, thread/start, turn/start) completes in <1 s. `account/usage/read` logged out returns its auth error in <1 s. The 15 s / 30 s gaps in run4/run6 were the one-behind flush meeting the client's timeout rhythm |
| 5 | Server logs its internals to SQLite | `run*/codex-home/logs_2.sqlite`, table `logs(ts, level, target, feedback_log_body)` | Full dispatch trace per request (`app-server request: X connection_id=… request_id=…`), auth reloads, remote-control websocket state. When logged out it re-logs `waiting to resolve remote control preference until authentication is available` every second — noise, not a stall |
| 6 | Server exits cleanly on stdin EOF | closed stdin, waited | exit 0, no crash; thread state persists in `$CODEX_HOME` (see [#12](#)) |
| 7 | Unknown method error lists the whole surface | sent `no/such/method` | `-32600` (not -32601) `Invalid request: unknown variant `no/such/method`, expected one of …` with **all 192 method names inline** |
| 8 | 192 client request methods | [#7](#) | `initialize`, `thread/{start,resume,fork,sample,sample/cancel,archive,delete,unsubscribe,increment_elicitation,decrement_elicitation,name/set,prediction/request,goal/set,goal/get,goal/clear,queue/add,queue/list,queue/update,queue/delete,queue/reorder,queue/start,metadata/update,attachment/add,attachment/list,attachmentOwner/list,attachment/remove,section/move,settings/update,memoryMode/set,memory/read,unarchive,compact/start,shellCommand,approveGuardianDeniedAction,backgroundTerminals/{clean,list,terminate},revert,list}`, `threadSection/{list,create,update,delete}`, `thread/{search,search/stream,search/cancel,search/stream/cancel,searchOccurrences,loaded/list,read,readState/update,turns/list,items/list,items/read,guardianReview,inject_items,prepareApprovedPlan}`, `rollout/compress`, `memory/{status,reset}`, `project/{list,read,create,import,update,move,delete}`, `skills/{list,extraRoots/set,config/write}`, `hooks/list`, `marketplace/{add,remove,upgrade}`, `plugin/{list,search,installed,reconcile,read,skill/read,share/*,install,uninstall}`, `app/{read,list,installed}`, `fs/{readFile,writeFile,createDirectory,getMetadata,readDirectory,remove,copy,watch,unwatch}`, `turn/{start,settings/update,steer,interrupt}`, `thread/realtime/{start,appendAudio,appendText,appendSpeech,stop}`, `thread/{timeline/list,realtime/list,realtime/listVoices}`, `review/start`, `model/{list}`, `modelProvider/{list,config/write,default/set,capabilities/read}`, `experimentalFeature/{list,enablement/set}`, `remoteControl/{enable,disable,status/read,pairing/start,pairing/status,client/list,client/revoke}`, `collaborationMode/list`, `environment/{add,info,status}`, `mcpServer/{oauth/login,resource/read,event/stream/start,event/stream/stop,tool/call}`, `config/mcpServer/reload`, `mcpServerStatus/list`, `account/*` (14 incl. `login/start`, `usage/read`, `rateLimits/read`), `command/exec*`, `process/{spawn,writeStdin,kill,resizePty}`, `config/{read,value/write,batchWrite,requirements/read}`, `externalAgentConfig/*`, `windowsSandbox/*`, `feedback/{upload,save}`, `userVerification/*`, `server/diagnostics`, `getConversationSummary`, `gitDiffToRemote`, `getAuthStatus`, `fuzzyFileSearch*`, `mock/experimentalMethod` |
| 9 | 95 server→client notification methods | `schema/ServerNotification.json` (oneOf, enums per branch) | Item/turn/thread lifecycle (see [#14](#)), `account/*`, `remoteControl/status/changed`, `config/*`, `mcpServer/*`, `windowsSandbox/*`, `server/diagnostics` changed, `warning`, etc. |
| 10 | Server→client **requests** exist (10) | `schema/ServerRequest.json` | `item/commandExecution/*` approval flows and elicitation — the permission/ask channel. Not exercised in this probe (mock emits no tool calls) |
| 11 | Invalid params use -32600 too | `turn/start` with `threadId: null` / `"thread-not-valid"` | `invalid type: null, expected a string`; `invalid thread id: invalid character: expected an optional prefix of `urn:uuid:` followed by [0-9a-fA-F-], found `o`` — ids accept an optional `urn:uuid:` prefix; bare ULIDs work |

## Conversation surface

| # | Claim | How | Result |
|---|---|---|---|
| 12 | `thread/start` {cwd, approvalPolicy, sandboxPolicy, model} | sent | → `thread` object: `{id: <ULID>, environments: [{environmentId: "local", cwd, runtimeWorkspaceRoots}], sessionId, forkedFromId, attachedToThreadId, parentThreadId, preview, ephemeral, section, projectId, historyMode, …}`; then `thread/started` notification. **Threads persist across server restarts** — a fresh process lists and resumes threads from earlier processes (rollout files under `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl`) |
| 13 | `thread/list` {limit} / `thread/read` {threadId} | sent | list → `data: [thread]` with `preview` (first user text), `path` (rollout jsonl), `originator`, `cwd`, `model`, `modelProvider`, `createdAt/updatedAt/recencyAt`, `status: {type:"notLoaded"}`, `source: "vscode"` (set by the app-server itself), `cliVersion`. `thread/read` returns the same thread object; a live one has `environments` populated |
| 14 | Turn notification stream (mock provider, "Say hello") | `turn/start`, read stream | Exact order: `thread/status/changed` {`status: {type:"active", activeFlags:[]}`} → `turn/started` {threadId, turn} → `item/started` (userMessage, full text) → `item/completed` (userMessage) → `item/started` (agentMessage, `text: ""`) → `item/agentMessage/delta` {threadId, turnId, itemId, `delta`} ×N → `item/completed` (agentMessage, **full text**) → `turn/completed` {threadId, turn `{status:"completed", items:[…], startedAtMs, completedAtMs, durationMs}`}. Deltas carry no sequence number; `emittedAtMs` on every notification |
| 15 | Item types observed: `userMessage` {id, clientId, content: [{type:"text", text, text_elements}], startedAtMs, completedAtMs} and `agentMessage` {id, text, moderation, phase, memoryCitation, delivery, questions, startedAtMs, completedAtMs} | [#14](#) | Schema defines more (reasoning, toolCall, commandExecution, …) — not observed, mock emits text only |
| 16 | `turn/interrupt` {threadId, turnId} mid-turn | sent during a 3 s stream | → result `{}`. The in-flight agentMessage gets **no** `item/completed`; `turn/completed` arrives with `status: "interrupted"`, `items: []` |
| 17 | `turn/steer` {threadId, expectedTurnId, input} mid-turn | sent during a stream | → result `{turnId}` (same turn id). Stream: new `item/started`/`item/completed` userMessage ("Now say bye") injected, then the agentMessage is **restarted** (new `item/started` for the same provider item id, deltas from scratch) and the turn completes with the steered text. Steering restarts the agent's in-progress output, it does not append to it |
| 18 | `thread/turns/list` {threadId, limit} | sent after 3 turns | → `data: [turn]` each with `items` (userMessage + agentMessage incl. full text), `status`, `startedAtMs`, `completedAtMs`, `durationMs` — enough to rebuild history without the rollout file |
| 19 | `thread/items/list` {threadId, limit} | sent | → `data: [{turnId, item, startedAtMs, completedAtMs}]` flat across turns |
| 20 | `warning` notification for unknown model | `thread/start` with `model: "mock-model"` | `{"method":"warning","params":{"threadId":…,"message":"Model metadata for `mock-model` not found. Defaulting to fallback metadata; this can degrade performance and cause issues."}}` — a non-fatal warning channel, separate from turn errors |

## Account, model, config surface (logged out)

| # | Claim | How | Result |
|---|---|---|---|
| 21 | `model/list` → 11-model catalog | sent | `data: [{id, model, displayName, description, supportedReasoningEfforts, defaultReasoningEffort, inputModalities, isDefault, hidden, serviceTiers, upgrade, upgradeInfo, availabilityNux, modelSpecialty, …}]` (gpt-5.5 … gpt-6.2-sol). Served without auth |
| 22 | `account/read` logged out | sent | `{account: null, userId: null, requiresOpenaiAuth: false, workspaceRouting: null}` |
| 23 | `account/usage/read` logged out | sent | `-32600` `codex account authentication required to read token usage` |
| 24 | `config/read` → full live config | sent | ~100-key object: `model`, `model_provider`, `model_providers` (mock visible), `approval_policy`, `sandbox_mode`, `instructions`, `mcp_servers`, `features`, `permissions`, `realtime`, … — the source can read effective config per thread |

## Error surface

| # | Claim | How | Result |
|---|---|---|---|
| 25 | Provider 429 → turn fails, thread survives | mock configured to 429 every request | `turn/completed` with `status: "failed"`, `items: []`, `error: {message: "exceeded retry limit, last status: 429 Too Many Requests", codexErrorInfo: {responseTooManyFailedAttempts: {httpStatusCode: 429}}, additionalDetails: null, misalignment: null}`, `durationMs ≈ 90`. Subsequent turns on the same thread work |
| 26 | Rate-limit / quota details unmeasured | — | `account/rateLimits/read` and `account/usage/read` exist but need a logged-in account; not run (hard rule) |

## Not measured (needs a live, authenticated Codex session — deliberately not done)

- Real model streams: reasoning items, `toolCall`/`commandExecution` items, and the server→client
  approval/elicitation **requests** ([#10](#)) — the mock emits text only, so the ask/permission
  channel was never exercised end to end.
- `thread/fork` semantics, `thread/queue/*` (a pending-input queue with reorder),
  `thread/compact/start`, realtime (audio) turns, `review/start`, MCP server flows,
  `remoteControl` pairing (multi-client attachment to one server).
- Pagination beyond `limit` for `thread/turns/list` / `thread/items/list`.
- `Ending::ToTerminal` — the app-server is headless; codex's TUI is a different mode of the CLI,
  not a mode of an app-server thread. No hand-off path exists in this protocol.

## Trait mapping (`kampr-core/src/conversation/`)

Maps cleanly:

- `Source::create(NewSession{cwd, prompt})` → `thread/start` + `turn/start` ([#12](#), [#14](#))
- `Source::open(pane)` → `thread/resume` {threadId}; `Source::resumable(cwd)` → `thread/list`
  (`preview` → `Stored.title`, `cwd` → `Stored.cwd`) ([#12](#), [#13](#))
- `Command::Input` (when = now, idle thread) → `turn/start`; queued variants → `thread/queue/add`
  (not exercised)
- `Command::Steer(Steer::Interrupt)` → `turn/interrupt` ([#16](#)); mid-turn text steer →
  `turn/steer` ([#17](#))
- `Command::Older` / `Update::Turns` / `Update::Page` → `thread/turns/list` + `thread/items/list`
  ([#18](#), [#19](#))
- `Command::Find` → `thread/search` / `thread/search/stream` (exist, not exercised)
- `Update::Live.text` → `item/agentMessage/delta`; `Update::State` → `thread/status/changed` +
  turn `status` ([#14](#))
- `Update::Trouble` → `turn/completed status:"failed"` + `error` ([#25](#)); `Update::Notice` →
  `warning` notifications ([#20](#))
- `Update::Session` → `account/read` + `config/read` ([#22](#), [#24](#))
- `Update::Asks` / `Command::Answer` → server→client approval/elicitation requests ([#10](#)) —
  shape known from schema, unexercised
- `Update::LiveTool` → `toolCall` items (schema only, unobserved)

Does NOT map (what the trait assumed about Claude/muffit that Codex breaks):

1. **No capabilities in the handshake.** `initialize` returns no capabilities ([#2](#));
   `Update::Abilities` has to be inferred from the 192-method list and `config/read`. Claude's
   `--output-format`/muffit's `commands.list` both declare; Codex does not.
2. **The transport forces pipelining** ([#3](#)). The trait's `run` loop (send `Command`, emit
   `Update`) models request/response; the app-server stdio transport deadlocks a client that waits
   for one response before sending the next. The source needs a background nudge or a permanent
   request in flight — a property the trait has no seam for.
3. **Turns are first-class, messages are items inside them.** Claude/muffit streams are
   message-centric (blocks in a turn); Codex is item-centric with `turnId`/`rootTurnId` trees
   (steer re-roots, fork branches). `Update::Turns(Vec<Turn>)` fits; there is no seam for
   `rootTurnId` ancestry or for "the turn restarted in place" ([#17](#)).
4. **A pending-input queue** (`thread/queue/{add,list,update,reorder,start}`) — richer than
   `When::{Idle,Next,Now}`; the trait has no queue-inspection or reorder command.
5. **Multi-client / remote control** (`remoteControl/*`, `thread/unsubscribe`) — one server,
   many attachers, with pairing. The trait assumes one client per conversation.
6. **No `ToTerminal`** (see Not measured) — `Ending::ToTerminal` is unfillable for this source.
7. **`Update::Composer`, `Update::Suggested`, `Update::ClaudeHooks`, `Update::Tasks`/`TaskOutput`**
   have no counterpart (closest: `hooks/list`, `thread/backgroundTerminals/*`, `fuzzyFileSearch*`).
8. **Huge non-conversation surface** — projects, plugins, marketplace, MCP servers, filesystem,
   process spawn, realtime audio, goals, memory: 192 methods where the trait's pane needs ~15.