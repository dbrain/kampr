package dev.kampr.shared.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.paneTitle
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.util.bypassesSafety
import dev.kampr.shared.util.commandLine
import dev.kampr.shared.util.parseArgs
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.SessionInfo
import dev.kampr.shared.wire.SplitDirection
import dev.kampr.shared.wire.workspaceIdOf

private enum class Step { Menu, Workspace, Tab, Worktree, Session, Node }

private enum class Pick { Workspace, Split, Agent }

private const val AGENT_CHIPS = 5
private val RATIOS = listOf("⅓" to 0.33, "½" to 0.5, "⅔" to 0.67)

// Herdr puts a named session under its config root and on a command line, so the node validates
// the name; refusing it here means the operator sees why instead of eating a `bad_request`.
private val SESSION_NAME = Regex("^[A-Za-z0-9_-]{1,64}$")

// A shell alias cannot be started by `agent.start` — an alias only exists inside an interactive
// shell — but the argv behind one can, and the node has always forwarded it. Somebody who wants
// `--dangerously-skip-permissions` wants it every launch, so it is kept per harness rather than
// retyped; the sheet prints the resulting command line so a remembered flag is never a silent one.
interface AgentArgs {
    fun get(kind: String): String
    fun remember(kind: String, text: String?)
}

object NoAgentArgs : AgentArgs {
    override fun get(kind: String): String = ""
    override fun remember(kind: String, text: String?) = Unit
}

@Composable
fun NewSheet(
    breakpoint: Breakpoint,
    node: NodeInfo,
    pane: PaneInfo?,
    nodes: List<NodeInfo>,
    caps: ServerMsg.NodeCaps?,
    outcome: ServerMsg.Managed?,
    onManage: (ManageOp) -> Unit,
    onNode: (String) -> Unit,
    onNodePicker: () -> Unit,
    onDismiss: () -> Unit,
    panes: List<PaneInfo> = emptyList(),
    onRefreshCaps: () -> Unit = {},
    agentArgs: AgentArgs = NoAgentArgs,
) {
    val tokens = Kampr.tokens
    var step by remember { mutableStateOf(Step.Menu) }
    var pick by remember { mutableStateOf(Pick.Workspace) }
    var direction by remember { mutableStateOf(SplitDirection.Right) }
    var ratio by remember { mutableStateOf(0.5) }
    var kind by remember { mutableStateOf<String?>(null) }
    var allKinds by remember { mutableStateOf(false) }
    var label by remember { mutableStateOf(TextFieldValue()) }
    var cwd by remember(pane?.cwd) { mutableStateOf(TextFieldValue(pane?.cwd.orEmpty())) }
    var branch by remember { mutableStateOf(TextFieldValue()) }
    var base by remember { mutableStateOf(TextFieldValue()) }
    var sessionName by remember { mutableStateOf(TextFieldValue()) }
    var worktreePath by remember { mutableStateOf(TextFieldValue()) }
    var existingWorktree by remember { mutableStateOf(false) }
    var agentPane by remember(node.id) { mutableStateOf<String?>(null) }
    var agentName by remember { mutableStateOf(TextFieldValue()) }
    var agentFlags by remember { mutableStateOf(TextFieldValue()) }
    var keepFlags by remember { mutableStateOf(true) }
    val env = remember { mutableStateListOf<Pair<String, String>>() }
    var inFlight by remember { mutableStateOf<String?>(null) }
    var refusal by remember { mutableStateOf<String?>(null) }

    val peers = nodes.filter { it.id != node.id }
    // An agent starts *in* a pane, and the herd's own + opens this sheet without one. The panes
    // to offer are the ones on the machine the sheet is aimed at: `caps.agentKinds` is that
    // node's answer and a peer's harnesses are its own.
    val ownPanes = if (pane != null) emptyList() else panes.filter { it.nodeId == node.id }
    val agentTarget = pane ?: ownPanes.firstOrNull { it.id == agentPane }
    val kinds = caps?.agentKinds.orEmpty()
    LaunchedEffect(kinds) {
        if (kind == null) kind = kinds.firstOrNull()
    }
    LaunchedEffect(kind) {
        val chosen = kind ?: return@LaunchedEffect
        agentFlags = TextFieldValue(agentArgs.get(chosen))
        keepFlags = true
    }

    // The node is authoritative: the sheet closes on its ack, and the herd redraws from the
    // `herd.patch` that follows — never from anything guessed here.
    //
    // A named session is the exception, and it is why "session doesn't open when done" and
    // "doesn't close when done" were the same report twice. Everything else this sheet makes is
    // a pane or a container, and closing reveals it behind the sheet; a session is its own herdr
    // server that joins the herd as a node with no panes at all, and the only surface that shows
    // one is this sheet's own list. Closing on the ack took the operator away from the single
    // place the result was about to appear — and it appears only if something re-asks, because
    // the list is `caps.sessions` and the node caches that answer.
    LaunchedEffect(outcome) {
        val ack = outcome ?: return@LaunchedEffect
        if (ack.op != inFlight) return@LaunchedEffect
        inFlight = null
        when {
            !ack.ok -> refusal = ack.message ?: ack.code
            ack.op.startsWith("session.") -> {
                sessionName = TextFieldValue()
                onRefreshCaps()
            }
            else -> onDismiss()
        }
    }

    fun run(op: ManageOp) {
        refusal = null
        inFlight = op.op
        onManage(op)
    }

    val nodeId = node.id
    val workspaceId = pane?.workspaceId ?: pane?.id?.let(::workspaceIdOf)
    val trimmedCwd = cwd.text.trim().ifEmpty { null }
    val trimmedLabel = label.text.trim().ifEmpty { null }

    val action: Pair<String, (() -> Unit)?> = when (step) {
        Step.Menu -> when (pick) {
            Pick.Workspace -> "Create workspace" to {
                run(ManageOp.WorkspaceCreate(nodeId, cwd = trimmedCwd))
            }
            Pick.Split -> "Split ${direction.wire}" to (pane?.let { p ->
                { run(ManageOp.PaneSplit(p.id, direction, ratio, trimmedCwd)) }
            })
            Pick.Agent -> "Start ${kind ?: "an agent"}" to (
                if (agentTarget != null && kind != null) {
                    {
                        val chosen = kind!!
                        val typed = agentFlags.text.trim()
                        agentArgs.remember(chosen, if (keepFlags) typed.ifEmpty { null } else null)
                        run(
                            ManageOp.AgentStart(
                                agentTarget.id,
                                chosen,
                                agentName.text.trim().ifEmpty { null },
                                parseArgs(typed),
                            )
                        )
                    }
                } else null
                )
        }
        Step.Workspace -> "Create workspace" to {
            run(ManageOp.WorkspaceCreate(nodeId, trimmedLabel, trimmedCwd, env.filter { it.first.isNotBlank() }.toMap()))
        }
        Step.Tab -> "Create tab" to (workspaceId?.let { at ->
            { run(ManageOp.TabCreate(at, trimmedLabel, trimmedCwd)) }
        })
        Step.Worktree -> if (existingWorktree) {
            "Open worktree" to (worktreePath.text.trim().ifEmpty { null }?.let { path ->
                { run(ManageOp.WorktreeOpen(nodeId, path, trimmedCwd, trimmedLabel)) }
            })
        } else {
            "Create worktree" to (branch.text.trim().ifEmpty { null }?.let { b ->
                { run(ManageOp.WorktreeCreate(nodeId, b, base.text.trim().ifEmpty { null }, trimmedCwd, trimmedLabel)) }
            })
        }
        Step.Session -> "Start session" to (sessionName.text.trim().takeIf { SESSION_NAME.matches(it) }?.let { n ->
            { run(ManageOp.SessionCreate(nodeId, n)) }
        })
        // The rows are the action: picking one is the whole step, and a button under them would
        // be one that is never the thing to press.
        Step.Node -> "" to null
    }

    BottomSheet(breakpoint, onDismiss) {
        SheetHeader(
            title = when (step) {
                Step.Menu -> "New"
                Step.Workspace -> "Workspace"
                Step.Tab -> "Tab"
                Step.Worktree -> "Worktree"
                Step.Session -> "Named session"
                Step.Node -> "Machine"
            },
            subtitle = listOfNotNull("on ${node.name}", pane?.workspace).joinToString(" · "),
            onBack = if (step == Step.Menu) null else ({ step = Step.Menu; refusal = null }),
            onClose = onDismiss,
            compact = breakpoint == Breakpoint.Landscape,
        )

        Column(Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState())) {
            when (step) {
                Step.Menu -> Menu(
                    breakpoint = breakpoint,
                    node = node,
                    pane = pane,
                    peers = peers,
                    kinds = kinds,
                    allKinds = allKinds,
                    kind = kind,
                    pick = pick,
                    direction = direction,
                    ratio = ratio,
                    sessions = caps?.sessions.orEmpty(),
                    onStep = { step = it; refusal = null },
                    onPick = { pick = it },
                    onDirection = { direction = it; pick = Pick.Split },
                    onRatio = { ratio = it },
                    onKind = { kind = it; pick = Pick.Agent },
                    onMoreKinds = { allKinds = true },
                    agentPanes = ownPanes,
                    agentTarget = agentTarget,
                    agentPane = agentPane,
                    onAgentPane = { agentPane = it; pick = Pick.Agent },
                    agentName = agentName,
                    onAgentName = { agentName = it },
                    agentFlags = agentFlags,
                    onAgentFlags = { agentFlags = it },
                    keepFlags = keepFlags,
                    onKeepFlags = { keepFlags = it },
                    onNodePicker = onNodePicker,
                )
                Step.Node -> Fields {
                    for (machine in nodes) {
                        SheetCard(
                            icon = null,
                            iconTint = null,
                            title = machine.name,
                            subtitle = when {
                                !machine.online -> machine.detail ?: "unreachable"
                                machine.id == node.id -> "what this sheet is aimed at"
                                machine.kind == "local" -> "the node this device is connected to"
                                else -> "a paired machine"
                            },
                            selected = machine.id == node.id,
                            compact = breakpoint == Breakpoint.Landscape,
                            onClick = if (!machine.online) null else ({
                                onNode(machine.id)
                                step = Step.Menu
                                refusal = null
                            }),
                            label = "Create on ${machine.name}",
                        )
                    }
                    SheetCard(
                        icon = null,
                        iconTint = null,
                        title = "Pair a machine",
                        subtitle = "add another node to this herd",
                        compact = breakpoint == Breakpoint.Landscape,
                        onClick = onNodePicker,
                    )
                    // A peer answers `caps` to its own clients, not to this one, so the agent
                    // kinds and named sessions belong to the node this device is connected to.
                    // Everything else in the sheet is addressed by id and reaches any of them.
                    Note("Workspaces, tabs, splits and worktrees can be made on any machine here. Agents and named sessions are offered by the node this device is connected to.")
                }
                Step.Workspace -> Fields {
                    LabelledField("label", "kampr", label) { label = it }
                    LabelledField("directory", "/home/dbrain/dev/kampr", cwd) { cwd = it }
                    EnvEditor(
                        env,
                        { index, row -> env[index] = row },
                        { env.add("" to "") },
                        { env.removeAt(it) },
                    )
                    Note("A workspace, its directory and its variables in one call — a Claude session in this worktree with these variables is one action, not a script.")
                }
                Step.Tab -> Fields {
                    LabelledField("label", "tests", label) { label = it }
                    LabelledField("directory", pane?.cwd ?: "/home/dbrain/dev/kampr", cwd) { cwd = it }
                    Note(
                        if (workspaceId == null) "Open a pane first — a tab is created inside a workspace."
                        else "In ${pane?.workspace ?: workspaceId}."
                    )
                }
                Step.Worktree -> Fields {
                    Segmented(
                        listOf("New branch", "Existing path"),
                        if (existingWorktree) 1 else 0,
                        { existingWorktree = it == 1 },
                        Modifier.fillMaxWidth(),
                    )
                    if (existingWorktree) {
                        LabelledField("path", "/home/dbrain/dev/kampr-feat-x", worktreePath) { worktreePath = it }
                    } else {
                        LabelledField("branch", "feat/mesh-auth", branch) { branch = it }
                        LabelledField("base", "main", base) { base = it }
                        LabelledField("repository", pane?.cwd ?: "/home/dbrain/dev/kampr", cwd) { cwd = it }
                    }
                    LabelledField("label", "mesh-auth", label) { label = it }
                    Note("Herdr's own git support: a worktree for the branch, and a workspace opened on it.")
                }
                Step.Session -> Fields {
                    LabelledField("name", "agents", sessionName) { sessionName = it }
                    if (sessionName.text.isNotEmpty() && !SESSION_NAME.matches(sessionName.text.trim())) {
                        Note("Letters, digits, - and _ only, up to 64 — the name becomes a directory and a command line.")
                    }
                    SessionList(caps?.sessions.orEmpty()) { run(ManageOp.SessionStop(nodeId, it)) }
                    Note("A named session is its own Herdr server with its own socket, so the node starts it by running the CLI rather than calling a method. It joins the herd as another node.")
                }
            }
            Box(Modifier.height(6.dp))
        }

        Column(
            Modifier.padding(start = 16.dp, end = 16.dp, top = 10.dp, bottom = 18.dp),
            verticalArrangement = Arrangement.spacedBy(9.dp),
        ) {
            if (step == Step.Menu && pick == Pick.Split) {
                KText(
                    "A split changes the Herdr layout, so the desk and every other viewer get the new shape too.",
                    tokens.type.captionSmall,
                    tokens.color.working,
                    Modifier.announce(
                        "A split changes the Herdr layout, so the desk and every other viewer get the new shape too.",
                    ),
                    maxLines = 3,
                )
            }
            // A "Start claude" that cannot be pressed, with the reason a scroll away in the card
            // above it, is what the operator read as a broken button. The reason belongs beside
            // the button that is refusing.
            if (step == Step.Menu && pick == Pick.Agent && agentTarget == null) {
                val why = if (ownPanes.isEmpty()) {
                    "There is no pane on ${node.name} to start ${kind ?: "an agent"} in — make a workspace first."
                } else {
                    "Pick the pane ${kind ?: "the agent"} starts in."
                }
                KText(
                    why,
                    tokens.type.captionSmall,
                    tokens.color.working,
                    Modifier.announce(why),
                    maxLines = 3,
                )
            }
            refusal?.let {
                KText(it, tokens.type.captionSmall, tokens.color.blocked, Modifier.announce(it, urgent = true), maxLines = 3)
            }
            if (step != Step.Node) {
                PrimaryAction(
                    text = if (inFlight != null) "Waiting for the node" else action.first,
                    onClick = { action.second?.invoke() },
                    modifier = Modifier.fillMaxWidth(),
                    enabled = inFlight == null && action.second != null,
                )
            }
        }
    }
}

@Composable
private fun Fields(content: @Composable () -> Unit) {
    Column(
        Modifier.padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(11.dp),
    ) { content() }
}

@Composable
private fun Note(text: String) {
    val tokens = Kampr.tokens
    KText(text, tokens.type.captionSmall, tokens.color.mute, maxLines = 4)
}

@Composable
private fun Menu(
    breakpoint: Breakpoint,
    node: NodeInfo,
    pane: PaneInfo?,
    peers: List<NodeInfo>,
    kinds: List<String>,
    allKinds: Boolean,
    kind: String?,
    pick: Pick,
    direction: SplitDirection,
    ratio: Double,
    sessions: List<SessionInfo>,
    onStep: (Step) -> Unit,
    onPick: (Pick) -> Unit,
    onDirection: (SplitDirection) -> Unit,
    onRatio: (Double) -> Unit,
    onKind: (String) -> Unit,
    onMoreKinds: () -> Unit,
    agentPanes: List<PaneInfo>,
    agentTarget: PaneInfo?,
    agentPane: String?,
    onAgentPane: (String) -> Unit,
    agentName: TextFieldValue,
    onAgentName: (TextFieldValue) -> Unit,
    agentFlags: TextFieldValue,
    onAgentFlags: (TextFieldValue) -> Unit,
    keepFlags: Boolean,
    onKeepFlags: (Boolean) -> Unit,
    onNodePicker: () -> Unit,
) {
    val compact = breakpoint == Breakpoint.Landscape
    val agent = AgentPick(
        kinds, allKinds, kind, agentName, onAgentName, agentFlags, onAgentFlags, keepFlags,
        onKeepFlags, onKind, onMoreKinds, node.name, agentPanes, agentTarget, agentPane, onAgentPane,
    )
    if (breakpoint == Breakpoint.Portrait) {
        Column {
            Structure(compact, node, peers, pane, pick, direction, ratio, onStep, onPick, onDirection, onRatio)
            Elsewhere(compact, pane, peers, pick, sessions, onStep, agent, onNodePicker)
        }
    } else {
        Row(horizontalArrangement = Arrangement.spacedBy(0.dp)) {
            Column(Modifier.weight(1f)) {
                Structure(compact, node, peers, pane, pick, direction, ratio, onStep, onPick, onDirection, onRatio)
            }
            Column(Modifier.weight(1f)) {
                Elsewhere(compact, pane, peers, pick, sessions, onStep, agent, onNodePicker)
            }
        }
    }
}

// Eleven parameters about one card, threaded through two layouts, was the alternative.
private class AgentPick(
    val kinds: List<String>,
    val allKinds: Boolean,
    val kind: String?,
    val name: TextFieldValue,
    val onName: (TextFieldValue) -> Unit,
    val flags: TextFieldValue,
    val onFlags: (TextFieldValue) -> Unit,
    val keep: Boolean,
    val onKeep: (Boolean) -> Unit,
    val onKind: (String) -> Unit,
    val onMoreKinds: () -> Unit,
    val nodeName: String,
    val panes: List<PaneInfo>,
    val target: PaneInfo?,
    val paneId: String?,
    val onPane: (String) -> Unit,
)

@Composable
private fun Structure(
    compact: Boolean,
    node: NodeInfo,
    peers: List<NodeInfo>,
    pane: PaneInfo?,
    pick: Pick,
    direction: SplitDirection,
    ratio: Double,
    onStep: (Step) -> Unit,
    onPick: (Pick) -> Unit,
    onDirection: (SplitDirection) -> Unit,
    onRatio: (Double) -> Unit,
) {
    val tokens = Kampr.tokens
    Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(9.dp)) {
        // First, because it governs every card under it: everything this sheet makes is made on
        // one machine, and the herd's own New button has to aim somewhere before you have a pane
        // to aim it from. Buried at the bottom it read as "it always uses the first server".
        SheetCard(
            icon = KamprIcons.nodes,
            iconTint = tokens.color.dim,
            title = node.name,
            subtitle = if (peers.isEmpty()) "the only machine in this herd"
            else "made here — tap for the other ${peers.size}",
            compact = true,
            onClick = { onStep(Step.Node) },
            label = "Change machine, currently ${node.name}",
        )
        SheetCard(
            icon = KamprIcons.workspace,
            iconTint = tokens.color.accent,
            title = "Workspace",
            subtitle = "a directory and a fresh shell",
            compact = compact,
            selected = pick == Pick.Workspace,
            onClick = { onPick(Pick.Workspace); onStep(Step.Workspace) },
        )
        SheetCard(
            icon = KamprIcons.tab,
            iconTint = tokens.color.accent,
            title = "Tab",
            subtitle = pane?.workspace?.let { "another tab in $it" } ?: "open a pane first",
            compact = compact,
            onClick = if (pane == null) null else ({ onStep(Step.Tab) }),
        )
        SheetCard(
            icon = KamprIcons.split,
            iconTint = tokens.color.accent,
            title = "Split this pane",
            subtitle = if (pane == null) "open a pane first" else "changes the layout at the desk too",
            compact = compact,
            selected = pick == Pick.Split,
            trailing = if (pane == null) ({}) else ({
                Row(horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                    Chip(
                        "right", direction == SplitDirection.Right, { onDirection(SplitDirection.Right) },
                        quiet = true, label = "Split to the right",
                    )
                    Chip(
                        "down", direction == SplitDirection.Down, { onDirection(SplitDirection.Down) },
                        quiet = true, label = "Split downwards",
                    )
                }
            }),
        )
        if (pane != null && pick == Pick.Split) {
            Row(
                Modifier.padding(start = 4.dp),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                LabelText("ratio", tokens.type.micro, tokens.color.mute, Modifier.padding(top = 8.dp))
                RATIOS.forEach { (text, value) ->
                    Chip(text, ratio == value, { onRatio(value) }, quiet = true, label = "Split ratio $text")
                }
            }
        }
    }
}

@Composable
private fun Elsewhere(
    compact: Boolean,
    pane: PaneInfo?,
    peers: List<NodeInfo>,
    pick: Pick,
    sessions: List<SessionInfo>,
    onStep: (Step) -> Unit,
    agent: AgentPick,
    onNodePicker: () -> Unit,
) {
    val tokens = Kampr.tokens
    SheetSection("start an agent", compact)
    Box(Modifier.padding(horizontal = 16.dp)) {
        Surface(Modifier.fillMaxWidth()) {
            Column(
                Modifier.padding(horizontal = 15.dp, vertical = 13.dp),
                verticalArrangement = Arrangement.spacedBy(11.dp),
            ) {
                if (agent.kinds.isEmpty()) {
                    KText("The node has offered no agent kinds.", tokens.type.captionSmall, tokens.color.mute)
                } else {
                    val shown = if (agent.allKinds) agent.kinds else agent.kinds.take(AGENT_CHIPS)
                    FlowRow(
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                        verticalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        shown.forEach { k ->
                            Chip(k, k == agent.kind && pick == Pick.Agent, { agent.onKind(k) }, label = "Start a $k agent")
                        }
                        val rest = agent.kinds.size - shown.size
                        if (rest > 0) {
                            Chip("+$rest more", false, agent.onMoreKinds, quiet = true, label = "Show $rest more agent kinds")
                        }
                    }
                }
                if (pane == null && pick == Pick.Agent && agent.panes.isNotEmpty()) {
                    PanePick(agent)
                }
                val chosen = agent.kind
                if (agent.target != null && pick == Pick.Agent && chosen != null) {
                    KField("name it, or take the harness's own", agent.name, onChange = agent.onName)
                    AgentLaunch(chosen, agent)
                }
                KText(
                    when {
                        agent.target != null ->
                            "Offered by the node, not baked into the app — whatever Herdr can detect on that machine."
                        agent.panes.isEmpty() ->
                            "There is no pane on ${agent.nodeName} to start one in — make a workspace first."
                        else -> "An agent starts inside a pane. Pick the one on ${agent.nodeName} it runs in."
                    },
                    tokens.type.captionSmall,
                    tokens.color.mute,
                    maxLines = 3,
                )
            }
        }
    }

    SheetSection("from a branch", compact)
    Box(Modifier.padding(horizontal = 16.dp)) {
        SheetCard(
            icon = KamprIcons.branch,
            iconTint = tokens.color.done,
            title = "Worktree",
            subtitle = "a branch, its own directory, its own workspace",
            compact = compact,
            onClick = { onStep(Step.Worktree) },
        )
    }

    SheetSection("somewhere else", compact)
    Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(9.dp)) {
        SheetCard(
            icon = null,
            iconTint = null,
            title = "Named session",
            subtitle = sessions.takeIf { it.isNotEmpty() }
                ?.joinToString(", ") { it.name }
                ?.let { "its own server · $it" }
                ?: "its own server",
            subtitleMono = sessions.isNotEmpty(),
            compact = compact,
            onClick = { onStep(Step.Session) },
        )
    }
}

// The panes of the machine this sheet is aimed at, because `agent.start` takes a pane and the
// herd's + has none to give it. Titled the way the herd titles them, harness and all, so the pane
// already running a `claude` is recognisable as that before it is picked.
@Composable
private fun PanePick(agent: AgentPick) {
    val tokens = Kampr.tokens
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        LabelText("in which pane", tokens.type.micro, tokens.color.mute)
        FlowRow(
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            agent.panes.forEach { p ->
                val title = paneTitle(p)
                Chip(title, p.id == agent.paneId, { agent.onPane(p.id) }, label = "Start it in $title")
            }
        }
    }
}

// The launch, printed. A flag kept per harness is invisible by the second time it is used unless
// something says what is about to run — and one of these flags removes the confirmation an agent
// would otherwise ask for, which is not a thing to leave to memory.
@Composable
private fun AgentLaunch(kind: String, agent: AgentPick) {
    val tokens = Kampr.tokens
    val argv = parseArgs(agent.flags.text)
    val line = commandLine(kind, argv)
    val risky = argv.filter(::bypassesSafety)
    KField(
        "--flags for $kind",
        agent.flags,
        label = "Arguments for $kind",
        onChange = agent.onFlags,
    )
    Row(
        Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        KText(
            line,
            tokens.type.meta,
            if (risky.isEmpty()) tokens.color.text else tokens.color.blocked,
            Modifier.weight(1f).named("Starts $line"),
            maxLines = 3,
        )
        Chip(
            "remember",
            agent.keep,
            { agent.onKeep(!agent.keep) },
            quiet = true,
            label = "Remember these arguments for $kind",
        )
    }
    if (risky.isNotEmpty()) {
        val warning = risky.joinToString(" ") +
            " removes a confirmation step — this agent will act without asking"
        Row(
            Modifier
                .fillMaxWidth()
                .background(tokens.color.blockedBg, RoundedCornerShape(tokens.radii.sm))
                .announce(warning)
                .padding(horizontal = 10.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            IconGlyph(KamprIcons.warning, 13.dp, tokens.color.blocked)
            KText(warning, tokens.type.captionSmall, tokens.color.dim, maxLines = 3)
        }
    }
}

@Composable
private fun SessionList(sessions: List<SessionInfo>, onStop: (String) -> Unit) {
    val tokens = Kampr.tokens
    if (sessions.isEmpty()) return
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(5.dp)) {
        LabelText("on this host", tokens.type.micro, tokens.color.mute)
        sessions.forEach { session ->
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(9.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Mark(
                    if (session.running) tokens.color.done else tokens.color.idle,
                    if (session.running) MarkShape.Bar else MarkShape.Ring,
                    6.dp,
                )
                KText(
                    session.name,
                    tokens.type.meta,
                    if (session.served) tokens.color.text else tokens.color.mute,
                    Modifier.weight(1f).named(
                        "${session.name}, ${if (session.running) "running" else "stopped"}" +
                            if (session.served) "" else ", not served by this node",
                    ),
                )
                // An operator may restrict the set this node serves, and a session outside it
                // never joins the herd however healthy it looks here — so it is named as
                // somewhere no pane of this client's will ever open.
                if (!session.served) {
                    KText("not served", tokens.type.metaSmall, tokens.color.mute)
                }
                if (session.running) {
                    Chip("stop", false, { onStop(session.name) }, quiet = true, label = "Stop the ${session.name} session")
                }
            }
        }
    }
}
