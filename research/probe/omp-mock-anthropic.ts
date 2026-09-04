// A local endpoint that answers the Anthropic Messages API, so `omp` can be driven through its
// real code paths — streaming deltas, tool calls, spawns, approvals — with no credentials and no
// network. Started by hand beside research/probe/omp-screens.py:
//
//     bun research/probe/omp-mock-anthropic.ts     # listens on 127.0.0.1:8899
//
// The reply is scripted off what the request carries, never off a counter, so a retry or a
// resumed session lands on the same branch it did the first time. Keywords in the prompt pick the
// shape: SLOWCMD a 20 s command, FAILCMD a failing one, SPAWNANON a subagent with no name of its
// own, LONGANSWER a message long enough to be caught half-written.
const enc = (event: string, data: unknown) => `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`;

type Block = { type: "text"; text: string } | { type: "tool"; id: string; name: string; input: unknown };

function stream(blocks: Block[], stopReason: string) {
  let out = enc("message_start", {
    type: "message_start",
    message: { id: `msg_${Math.random().toString(36).slice(2)}`, type: "message", role: "assistant", model: "claude-sonnet-4-5", content: [], stop_reason: null, stop_sequence: null, usage: { input_tokens: 100, output_tokens: 1 } },
  });
  blocks.forEach((b, i) => {
    if (b.type === "text") {
      out += enc("content_block_start", { type: "content_block_start", index: i, content_block: { type: "text", text: "" } });
      for (const word of b.text.split(/(?<= )/)) out += enc("content_block_delta", { type: "content_block_delta", index: i, delta: { type: "text_delta", text: word } });
    } else {
      out += enc("content_block_start", { type: "content_block_start", index: i, content_block: { type: "tool_use", id: b.id, name: b.name, input: {} } });
      out += enc("content_block_delta", { type: "content_block_delta", index: i, delta: { type: "input_json_delta", partial_json: JSON.stringify(b.input) } });
    }
    out += enc("content_block_stop", { type: "content_block_stop", index: i });
  });
  out += enc("message_delta", { type: "message_delta", delta: { stop_reason: stopReason, stop_sequence: null }, usage: { output_tokens: 20 } });
  out += enc("message_stop", { type: "message_stop" });
  return out;
}

const MARK = "PROBE-SUBAGENT-TASK";

const LONG = [
  "A terminal emulator sits between a program writing bytes and a person reading glyphs,",
  "and almost everything hard about one lives in that gap. The program writes a stream with",
  "no idea how wide the screen is; the screen has a fixed grid and a cursor that the stream",
  "moves by escape sequence rather than by coordinate. Every wrap, every clear, every scroll",
  "is a decision the emulator makes on the program's behalf, and a wrong one is not a visual",
  "glitch — it is the wrong character under the cursor for the rest of the session.",
].join(" ");
let n = 0;
let subTurn = 0;

Bun.serve({
  port: 8899,
  idleTimeout: 0,
  async fetch(req) {
    const url = new URL(req.url);
    if (url.pathname.endsWith("count_tokens")) return Response.json({ input_tokens: 100 });
    const body = await req.text();
    const i = n++;
    let parsed: any = {};
    try { parsed = JSON.parse(body); } catch {}
    const messages = parsed.messages ?? [];
    const first = JSON.stringify(messages[0] ?? "");
    const isSub = first.includes(MARK);
    const turns = messages.filter((m: any) => m.role === "assistant").length;
    console.error(`req#${i} sub=${isSub} msgs=${messages.length} assistantTurns=${turns}`);

    const lastUser = JSON.stringify(messages[messages.length - 1] ?? "");
    // An image in the prompt: answer in one turn, so the transcript is the user record and nothing
    // else — which is what a probe of the record's shape wants.
    const carriesImage = body.includes('"type":"image"') || body.includes('"media_type"');
    let payload: string;
    const say = lastUser.match(/SAY: ([a-z ]+)/);
    if (!isSub && say) {
      // One plain answer per prompt, so a session can be walked forward and rewound with nothing
      // else moving in it.
      payload = stream([{ type: "text", text: `Answering ${say[1]}.` }], "end_turn");
    } else if (!isSub && carriesImage) {
      payload = stream([{ type: "text", text: "A picture, and I have looked at it." }], "end_turn");
    } else if (!isSub && lastUser.includes("SLOWCMD")) {
      payload = stream([
        { type: "text", text: "Running a slow command now." },
        { type: "tool", id: `toolu_slow${i}`, name: "bash", input: { command: "sleep 20; echo slow-done" } },
      ], "tool_use");
    } else if (!isSub && lastUser.includes("FAILCMD")) {
      payload = stream([
        { type: "text", text: "This one will fail." },
        { type: "tool", id: `toolu_fail${i}`, name: "bash", input: { command: "ls /no/such/path" } },
      ], "tool_use");
    } else if (!isSub && (lastUser.includes("EDITME") || body.includes("[README.md#"))) {
      // Hashline edits are anchored: the section tag has to be copied from the `read` that
      // produced it, so the read comes first and its own result carries the tag.
      const tag = body.match(/\[README\.md#([0-9A-F]{4})\]/);
      payload = tag
        ? stream([
            { type: "text", text: "Now I will change that line." },
            {
              type: "tool", id: `toolu_edit${i}`, name: "edit",
              input: {
                input: lastUser.includes("INSERT") || body.includes("INSERT")
                  ? `[README.md#${tag[1]}]\nPUT <3:\n+an inserted line\n+and another\n`
                  : lastUser.includes("TWOPLACES") || body.includes("TWOPLACES")
                  ? `[README.md#${tag[1]}]\nPUT 2.=2:\n+second line, changed\nPUT 8.=8:\n+eighth line, changed\n`
                  : `[README.md#${tag[1]}]\nPUT 1.=1:\n+hello from an edited omp\n`,
              },
            },
          ], "tool_use")
        : stream([
            { type: "text", text: "Reading the file first." },
            { type: "tool", id: `toolu_read${i}`, name: "read", input: { path: "README.md" } },
          ], "tool_use");
    } else if (!isSub && lastUser.includes("ASKTWO")) {
      // Two questions in one call, the first taking several answers: the shape that tells a client
      // a press is a tick rather than an answer, and the shape that keeps a pane blocked after the
      // first question has been answered.
      payload = stream([
        { type: "text", text: "Two things before I go on." },
        {
          type: "tool", id: `toolu_asktwo${i}`, name: "ask",
          input: {
            questions: [
              {
                id: "suites", question: "Which test suites should I run?", multi: true,
                options: [
                  { label: "unit", description: "The unit suite." },
                  { label: "integration", description: "The integration suite." },
                  { label: "browser", description: "The browser suite." },
                ],
              },
              {
                id: "branch", question: "Which branch should this land on?",
                options: [
                  { label: "main", description: "Straight onto the default branch." },
                  { label: "a topic branch", description: "Open a PR instead." },
                ],
              },
            ],
          },
        },
      ], "tool_use");
    } else if (!isSub && lastUser.includes("ASKME")) {
      payload = stream([
        { type: "text", text: "I need a decision before I go on." },
        {
          type: "tool", id: `toolu_ask${i}`, name: "ask",
          input: {
            questions: [{
              id: "branch",
              question: "Which branch should this land on?",
              options: [
                { label: "main", description: "Straight onto the default branch." },
                { label: "a topic branch", description: "Open a PR instead." },
              ],
            }],
          },
        },
      ], "tool_use");
    } else if (!isSub && lastUser.includes("LONGANSWER")) {
      // Long enough to be caught half-written: a preview is only published for a block that grows
      // between two polls, so a one-shot reply exercises nothing.
      payload = stream([{ type: "text", text: LONG }], "end_turn");
    } else if (!isSub && lastUser.includes("SPAWNANON")) {
      payload = stream([
        { type: "text", text: "Spawning an unnamed agent." },
        { type: "tool", id: `toolu_anon${i}`, name: "task", input: { agent: "task", task: `${MARK}: unnamed spawn, read the README` } },
      ], "tool_use");
    } else if (isSub) {
      subTurn++;
      payload = subTurn === 1
        ? stream([
            { type: "text", text: "Subagent starting: I will read the README." },
            { type: "tool", id: `toolu_sub${subTurn}`, name: "read", input: { path: "README.md" } },
          ], "tool_use")
        : stream([{ type: "text", text: "Subagent report: the README says hello world." }], "end_turn");
    } else if (turns === 0) {
      payload = stream([
        { type: "text", text: "I will run one command first." },
        { type: "tool", id: "toolu_probe1", name: "bash", input: { command: "echo hello-from-omp" } },
      ], "tool_use");
    } else if (turns === 1) {
      payload = stream([
        { type: "text", text: "Now I will fan out a subagent." },
        { type: "tool", id: "toolu_probe2", name: "task", input: { agent: "task", name: "prober", task: `${MARK}: read the README and report what it says` } },
      ], "tool_use");
    } else if (turns === 2) {
      payload = stream([
        { type: "text", text: "Waiting while the subagent works." },
        { type: "tool", id: "toolu_probe3", name: "bash", input: { command: "sleep 25; echo waited" } },
      ], "tool_use");
    } else {
      payload = stream([{ type: "text", text: "All done, the probe is complete." }], "end_turn");
    }
    const chunks = payload.split(/(?<=\n\n)/);
    const sse = new ReadableStream({
      async start(controller) {
        const te = new TextEncoder();
        for (const chunk of chunks) {
          controller.enqueue(te.encode(chunk));
          if (chunk.includes("text_delta")) await Bun.sleep(250);
        }
        controller.close();
      },
    });
    return new Response(sse, { headers: { "content-type": "text/event-stream" } });
  },
});
console.error("mock listening on 8899");
