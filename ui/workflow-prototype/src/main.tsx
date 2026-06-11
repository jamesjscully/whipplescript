import { render } from "@solidjs/web";
import { For, Show, createMemo, createSignal } from "solid-js";
import {
  Activity,
  AlertTriangle,
  Boxes,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  Clock3,
  Database,
  GitBranch,
  KeyRound,
  PauseCircle,
  Play,
  RefreshCw,
  Route,
  Search,
  Server,
  Settings,
  TimerReset,
  Workflow,
  XCircle
} from "./icons";
import type { IconComponent } from "./icons";
import "./styles.css";

type InstanceStatus = "running" | "paused" | "completed" | "failed" | "cancelled";
type NodeStatus =
  | "active"
  | "fired"
  | "completed"
  | "queued"
  | "running"
  | "blocked"
  | "failed"
  | "cancelled"
  | "skipped"
  | "not-started";
type NodeKind = "event" | "fact" | "rule" | "effect" | "terminal";
type View = "dashboard" | "detail" | "keys" | "settings";

type WorkflowNode = {
  id: string;
  label: string;
  detail: string;
  kind: NodeKind;
  status: NodeStatus;
  x: number;
  y: number;
  meta: string[];
};

type WorkflowEdge = {
  from: string;
  to: string;
  label: string;
  status: "solid" | "muted" | "warning" | "success";
};

type ScriptInstance = {
  id: string;
  workflow: string;
  source: string;
  status: InstanceStatus;
  owner: string;
  startedAt: string;
  updatedAt: string;
  queuedEffects: number;
  activeRuns: number;
  blockedEffects: number;
  failureCount: number;
  graph: {
    nodes: WorkflowNode[];
    edges: WorkflowEdge[];
  };
  events: Array<{
    time: string;
    type: string;
    summary: string;
    status: "ok" | "wait" | "error" | "info";
  }>;
};

const initialInstances: ScriptInstance[] = [
  {
    id: "ins_9e4c",
    workflow: "CoerceBranch",
    source: "examples/coerce-branch.whip",
    status: "running",
    owner: "ops@whipplescript.local",
    startedAt: "00:42:11",
    updatedAt: "00:57:09",
    queuedEffects: 1,
    activeRuns: 0,
    blockedEffects: 0,
    failureCount: 0,
    graph: {
      nodes: [
        {
          id: "started",
          label: "started",
          detail: "external.started",
          kind: "event",
          status: "completed",
          x: 20,
          y: 154,
          meta: ["source: external", "sequence: 1"]
        },
        {
          id: "work",
          label: "WorkItem",
          detail: "request fact",
          kind: "fact",
          status: "active",
          x: 140,
          y: 154,
          meta: ["title: Fix visualization", "status: active", "provenance: input"]
        },
        {
          id: "classify",
          label: "classify_request",
          detail: "rule",
          kind: "rule",
          status: "fired",
          x: 260,
          y: 154,
          meta: ["reads: WorkItem", "writes: ClassifiedMessage", "committed once"]
        },
        {
          id: "coerce",
          label: "classification",
          detail: "baml.coerce",
          kind: "effect",
          status: "completed",
          x: 395,
          y: 70,
          meta: ["kind: baml.coerce", "status: completed", "provider: fixture"]
        },
        {
          id: "fallback",
          label: "fallback review",
          detail: "human.ask",
          kind: "effect",
          status: "skipped",
          x: 395,
          y: 238,
          meta: ["only when classification fails", "status: not reached"]
        },
        {
          id: "classified",
          label: "ClassifiedMessage",
          detail: "derived fact",
          kind: "fact",
          status: "active",
          x: 520,
          y: 70,
          meta: ["priority: Normal", "confidence: 0.91", "provenance: rule"]
        },
        {
          id: "route",
          label: "route_classified",
          detail: "rule",
          kind: "rule",
          status: "fired",
          x: 520,
          y: 184,
          meta: ["reads: ClassifiedMessage", "committed once"]
        },
        {
          id: "routing",
          label: "routing review",
          detail: "human.ask",
          kind: "effect",
          status: "queued",
          x: 640,
          y: 184,
          meta: ["kind: human.ask", "status: queued", "waiting: operator"]
        }
      ],
      edges: [
        { from: "started", to: "work", label: "input", status: "success" },
        { from: "work", to: "classify", label: "read", status: "success" },
        { from: "classify", to: "coerce", label: "creates", status: "success" },
        { from: "coerce", to: "classified", label: "succeeds", status: "success" },
        { from: "coerce", to: "fallback", label: "fails", status: "muted" },
        { from: "classified", to: "route", label: "enables", status: "success" },
        { from: "route", to: "routing", label: "creates", status: "warning" }
      ]
    },
    events: [
      { time: "00:42:11", type: "external.started", summary: "Instance accepted start input.", status: "ok" },
      { time: "00:42:12", type: "rule.committed", summary: "classify_request created classification effect.", status: "ok" },
      { time: "00:42:13", type: "effect.terminal", summary: "classification completed through fixture provider.", status: "ok" },
      { time: "00:42:13", type: "fact.derived", summary: "ClassifiedMessage fact projected from coerce result.", status: "ok" },
      { time: "00:42:14", type: "rule.committed", summary: "route_classified enqueued routing review.", status: "wait" }
    ]
  },
  {
    id: "ins_f71a",
    workflow: "MultiAgentBoundedConcurrency",
    source: "examples/multi-agent-bounded-concurrency.whip",
    status: "running",
    owner: "automation@whipplescript.local",
    startedAt: "23:58:03",
    updatedAt: "00:55:30",
    queuedEffects: 3,
    activeRuns: 2,
    blockedEffects: 1,
    failureCount: 0,
    graph: {
      nodes: [
        { id: "ready", label: "WorkItem", detail: "ready fact", kind: "fact", status: "active", x: 25, y: 132, meta: ["count: 5"] },
        { id: "impl", label: "implement_ready_work", detail: "rule", kind: "rule", status: "fired", x: 165, y: 132, meta: ["capacity: 2"] },
        { id: "turn", label: "implementer turn", detail: "agent.tell", kind: "effect", status: "running", x: 320, y: 132, meta: ["profile: repo-writer", "runs: 2 active"] },
        { id: "complete", label: "completed turn", detail: "provider result", kind: "fact", status: "not-started", x: 475, y: 132, meta: ["waiting on agent output"] },
        { id: "review", label: "review_completed_turn", detail: "rule", status: "blocked", kind: "rule", x: 615, y: 132, meta: ["blocked by reviewer capacity"] }
      ],
      edges: [
        { from: "ready", to: "impl", label: "read", status: "success" },
        { from: "impl", to: "turn", label: "creates", status: "warning" },
        { from: "turn", to: "complete", label: "completes", status: "muted" },
        { from: "complete", to: "review", label: "enables", status: "muted" }
      ]
    },
    events: [
      { time: "23:58:03", type: "external.started", summary: "Instance created.", status: "ok" },
      { time: "23:58:07", type: "rule.committed", summary: "implement_ready_work enqueued five turns.", status: "ok" },
      { time: "00:12:19", type: "effect.blocked", summary: "One turn blocked by capacity.", status: "wait" },
      { time: "00:55:30", type: "effect.run_started", summary: "Two implementer turns running.", status: "info" }
    ]
  },
  {
    id: "ins_4d07",
    workflow: "MinimalNoop",
    source: "examples/minimal-noop.whip",
    status: "completed",
    owner: "local",
    startedAt: "00:57:09",
    updatedAt: "00:57:09",
    queuedEffects: 0,
    activeRuns: 0,
    blockedEffects: 0,
    failureCount: 0,
    graph: {
      nodes: [
        { id: "started", label: "started", detail: "external.started", kind: "event", status: "completed", x: 60, y: 140, meta: ["sequence: 1"] },
        { id: "observe", label: "observe_start", detail: "rule", kind: "rule", status: "fired", x: 250, y: 140, meta: ["trigger: started"] },
        { id: "seen", label: "StartupSeen", detail: "fact", kind: "fact", status: "active", x: 440, y: 140, meta: ["state: observed"] },
        { id: "done", label: "idle", detail: "fixed point", kind: "terminal", status: "completed", x: 630, y: 140, meta: ["no queued effects"] }
      ],
      edges: [
        { from: "started", to: "observe", label: "triggers", status: "success" },
        { from: "observe", to: "seen", label: "records", status: "success" },
        { from: "seen", to: "done", label: "idle", status: "success" }
      ]
    },
    events: [
      { time: "00:57:09", type: "external.started", summary: "Instance started.", status: "ok" },
      { time: "00:57:09", type: "rule.committed", summary: "observe_start recorded StartupSeen.", status: "ok" }
    ]
  }
];

const apiKeys = [
  { name: "Fixture provider", scope: "local validation", status: "healthy", lastUsed: "00:57" },
  { name: "Codex native", scope: "repo-writer turns", status: "missing", lastUsed: "never" },
  { name: "Claude native", scope: "review turns", status: "healthy", lastUsed: "yesterday" },
  { name: "BAML coerce", scope: "typed decisions", status: "rotating", lastUsed: "00:42" }
];

function App() {
  const [instances, setInstances] = createSignal<ScriptInstance[]>(initialInstances);
  const [view, setView] = createSignal<View>("dashboard");
  const [selectedId, setSelectedId] = createSignal(initialInstances[0].id);
  const [selectedNodeId, setSelectedNodeId] = createSignal<string | null>("routing");
  const [query, setQuery] = createSignal("");

  const selectedInstance = createMemo(
    () => instances().find((instance) => instance.id === selectedId()) ?? instances()[0]
  );
  const selectedNode = createMemo(() => {
    const id = selectedNodeId();
    return selectedInstance().graph.nodes.find((node) => node.id === id) ?? selectedInstance().graph.nodes[0];
  });
  const filteredInstances = createMemo(() => {
    const term = query().trim().toLowerCase();
    if (!term) return instances();
    return instances().filter(
      (instance) =>
        instance.workflow.toLowerCase().includes(term) ||
        instance.source.toLowerCase().includes(term) ||
        instance.id.toLowerCase().includes(term)
    );
  });

  const openInstance = (id: string) => {
    setSelectedId(id);
    const instance = instances().find((item) => item.id === id);
    setSelectedNodeId(instance?.graph.nodes.at(-1)?.id ?? null);
    setView("detail");
  };

  const transitionRun = (id: string, action: "pause" | "resume" | "cancel") => {
    setInstances((current) => current.map((instance) => transitionInstance(instance, id, action)));
  };

  return (
    <main class="shell">
      <Sidebar
        view={view()}
        setView={setView}
        instances={instances()}
        selectedId={selectedId()}
        openInstance={openInstance}
      />
      <section class="workspace">
        <TopBar query={query()} setQuery={setQuery} />
        <Show when={view() === "dashboard"}>
          <Dashboard
            instances={filteredInstances()}
            openInstance={openInstance}
            transitionRun={transitionRun}
          />
        </Show>
        <Show when={view() === "detail"}>
          <WorkflowDetail
            instance={selectedInstance()}
            node={selectedNode()}
            setView={setView}
            setSelectedNodeId={setSelectedNodeId}
            transitionRun={transitionRun}
          />
        </Show>
        <Show when={view() === "keys"}>
          <ApiKeys />
        </Show>
        <Show when={view() === "settings"}>
          <SettingsView />
        </Show>
      </section>
    </main>
  );
}

function transitionInstance(
  instance: ScriptInstance,
  id: string,
  action: "pause" | "resume" | "cancel"
): ScriptInstance {
  if (instance.id !== id || isTerminal(instance.status)) return instance;

  const time = currentTimeLabel();
  if (action === "pause" && instance.status === "running") {
    return {
      ...instance,
      status: "paused",
      updatedAt: time,
      events: [
        ...instance.events,
        { time, type: "instance.transitioned", summary: "Run paused. New rule commits are held.", status: "wait" }
      ]
    };
  }

  if (action === "resume" && instance.status === "paused") {
    return {
      ...instance,
      status: "running",
      updatedAt: time,
      events: [
        ...instance.events,
        { time, type: "instance.transitioned", summary: "Run resumed.", status: "info" }
      ]
    };
  }

  if (action === "cancel") {
    return {
      ...instance,
      status: "cancelled",
      updatedAt: time,
      queuedEffects: 0,
      activeRuns: 0,
      blockedEffects: 0,
      graph: {
        ...instance.graph,
        nodes: instance.graph.nodes.map((node) =>
          node.kind === "effect" && ["queued", "running", "blocked"].includes(node.status)
            ? {
                ...node,
                status: "cancelled",
                meta: node.meta.map((item) => (item.startsWith("status:") ? "status: cancelled" : item))
              }
            : node
        )
      },
      events: [
        ...instance.events,
        { time, type: "instance.transitioned", summary: "Run killed and remaining effects cancelled.", status: "error" }
      ]
    };
  }

  return instance;
}

function isTerminal(status: InstanceStatus) {
  return status === "completed" || status === "failed" || status === "cancelled";
}

function currentTimeLabel() {
  return new Date().toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false
  });
}

function Sidebar(props: {
  view: View;
  setView: (view: View) => void;
  instances: ScriptInstance[];
  selectedId: string;
  openInstance: (id: string) => void;
}) {
  const [expanded, setExpanded] = createSignal<Record<string, boolean>>({});
  const scripts = createMemo(() => {
    const byWorkflow = new Map<string, ScriptInstance[]>();
    for (const instance of props.instances) {
      byWorkflow.set(instance.workflow, [...(byWorkflow.get(instance.workflow) ?? []), instance]);
    }
    return Array.from(byWorkflow.entries()).map(([workflow, runs]) => ({ workflow, runs }));
  });
  const isExpanded = (workflow: string) => expanded()[workflow] ?? true;
  const toggleScript = (workflow: string) => {
    setExpanded((current) => ({ ...current, [workflow]: !isExpanded(workflow) }));
  };

  return (
    <aside class="sidebar">
      <div class="brand">
        <Workflow size={28} />
        <div>
          <strong>WhippleScript</strong>
          <span>Operations</span>
        </div>
      </div>
      <nav class="nav tree-nav">
        <button
          class={`nav-item ${props.view === "dashboard" ? "active" : ""}`}
          onClick={() => props.setView("dashboard")}
          title="All runs"
        >
          <Activity size={18} />
          <span>All runs</span>
        </button>
        <div class="tree-section">
          <span class="tree-label">Scripts</span>
          <For each={scripts()}>
            {(script) => (
              <div class="tree-branch">
                <button
                  class="script-node"
                  onClick={() => toggleScript(script.workflow)}
                  aria-expanded={isExpanded(script.workflow) ? "true" : "false"}
                  title={script.workflow}
                >
                  <ChevronRight class={`disclosure ${isExpanded(script.workflow) ? "expanded" : ""}`} size={16} />
                  <Workflow size={17} />
                  <span>{script.workflow}</span>
                  <em>{script.runs.length}</em>
                </button>
                <Show when={isExpanded(script.workflow)}>
                  <div class="run-children">
                    <For each={script.runs}>
                      {(run) => (
                        <button
                          class={`run-node ${props.selectedId === run.id && props.view === "detail" ? "active" : ""}`}
                          onClick={() => props.openInstance(run.id)}
                          title={`${run.workflow} ${run.id}`}
                        >
                          <span class={`status-dot ${run.status}`} />
                          <span>
                            <strong>{run.id}</strong>
                            <small>{run.status}</small>
                          </span>
                        </button>
                      )}
                    </For>
                  </div>
                </Show>
              </div>
            )}
          </For>
        </div>
        <div class="tree-section management">
          <span class="tree-label">Manage</span>
          <button
            class={`nav-item ${props.view === "keys" ? "active" : ""}`}
            onClick={() => props.setView("keys")}
            title="Provider keys"
          >
            <KeyRound size={18} />
            <span>Provider keys</span>
          </button>
          <button
            class={`nav-item ${props.view === "settings" ? "active" : ""}`}
            onClick={() => props.setView("settings")}
            title="Settings"
          >
            <Settings size={18} />
            <span>Settings</span>
          </button>
        </div>
      </nav>
    </aside>
  );
}

function TopBar(props: { query: string; setQuery: (value: string) => void }) {
  return (
    <header class="topbar">
      <div class="search">
        <Search size={17} />
        <input
          value={props.query}
          onInput={(event) => props.setQuery(event.currentTarget.value)}
          placeholder="Search workflows, instances, files"
        />
      </div>
      <div class="top-actions">
        <button class="soft-button">
          <RefreshCw size={16} />
          Sync
        </button>
        <button class="primary-button">
          <Play size={16} />
          New run
        </button>
      </div>
    </header>
  );
}

function Dashboard(props: {
  instances: ScriptInstance[];
  openInstance: (id: string) => void;
  transitionRun: (id: string, action: "pause" | "resume" | "cancel") => void;
}) {
  const totals = createMemo(() => ({
    running: props.instances.filter((item) => item.status === "running").length,
    activeRuns: props.instances.reduce((sum, item) => sum + item.activeRuns, 0),
    queued: props.instances.reduce((sum, item) => sum + item.queuedEffects, 0),
    blocked: props.instances.reduce((sum, item) => sum + item.blockedEffects, 0)
  }));
  return (
    <div class="page-grid">
      <section class="summary-row">
        <Metric icon={Activity} label="Running scripts" value={totals().running.toString()} tone="blue" />
        <Metric icon={Server} label="Active provider runs" value={totals().activeRuns.toString()} tone="green" />
        <Metric icon={Clock3} label="Queued effects" value={totals().queued.toString()} tone="amber" />
        <Metric icon={AlertTriangle} label="Blocked" value={totals().blocked.toString()} tone="red" />
      </section>
      <section class="panel wide">
        <div class="panel-heading">
          <div>
            <h1>Workflow Runs</h1>
            <p>Current instances, provider activity, and runtime pressure.</p>
          </div>
          <button class="soft-button">
            <TimerReset size={16} />
            Auto-refresh
          </button>
        </div>
        <div class="run-list">
          <For each={props.instances}>
            {(instance) => (
              <div
                class="run-row"
                role="button"
                tabindex={0}
                onClick={() => props.openInstance(instance.id)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") props.openInstance(instance.id);
                }}
              >
                <div class="run-main">
                  <StatusPill status={instance.status} />
                  <div>
                    <strong>{instance.workflow}</strong>
                    <span>{instance.source}</span>
                  </div>
                </div>
                <div class="run-counters">
                  <Counter label="queued" value={instance.queuedEffects} />
                  <Counter label="running" value={instance.activeRuns} />
                  <Counter label="blocked" value={instance.blockedEffects} />
                  <Counter label="failures" value={instance.failureCount} />
                </div>
                <div class="run-updated">
                  <span>{instance.id}</span>
                  <strong>{instance.updatedAt}</strong>
                </div>
                <RunActions instance={instance} transitionRun={props.transitionRun} />
              </div>
            )}
          </For>
        </div>
      </section>
      <section class="panel side-panel">
        <div class="panel-heading compact">
          <h2>Provider Keys</h2>
          <KeyRound size={18} />
        </div>
        <div class="key-health">
          <For each={apiKeys.slice(0, 3)}>
            {(key) => (
              <div class="key-item">
                <span class={`dot ${key.status}`} />
                <div>
                  <strong>{key.name}</strong>
                  <span>{key.scope}</span>
                </div>
              </div>
            )}
          </For>
        </div>
      </section>
    </div>
  );
}

function WorkflowDetail(props: {
  instance: ScriptInstance;
  node: WorkflowNode;
  setView: (view: View) => void;
  setSelectedNodeId: (id: string) => void;
  transitionRun: (id: string, action: "pause" | "resume" | "cancel") => void;
}) {
  return (
    <div class="detail-layout">
      <section class="detail-main">
        <div class="detail-header">
          <button class="soft-button" onClick={() => props.setView("dashboard")}>
            <ChevronLeft size={16} />
            Runs
          </button>
          <div>
            <h1>{props.instance.workflow}</h1>
            <p>
              {props.instance.id} · {props.instance.source}
            </p>
          </div>
          <div class="detail-actions">
            <StatusPill status={props.instance.status} />
            <RunActions instance={props.instance} transitionRun={props.transitionRun} compact />
          </div>
        </div>
        <WorkflowGraph instance={props.instance} setSelectedNodeId={props.setSelectedNodeId} selectedNodeId={props.node.id} />
        <EventTimeline instance={props.instance} />
      </section>
      <aside class="inspector">
        <div class="panel-heading compact">
          <h2>Inspector</h2>
          <NodeIcon kind={props.node.kind} />
        </div>
        <div class={`node-card ${props.node.kind} ${props.node.status}`}>
          <strong>{props.node.label}</strong>
          <span>{props.node.detail}</span>
        </div>
        <div class="inspector-section">
          <h3>Status</h3>
          <StatusBadge status={props.node.status} />
        </div>
        <div class="inspector-section">
          <h3>Fields</h3>
          <For each={props.node.meta}>
            {(item) => <div class="meta-line">{item}</div>}
          </For>
        </div>
        <div class="inspector-section">
          <h3>Runtime Actions</h3>
          <div class="action-grid">
            <button
              class="soft-button"
              disabled={isTerminal(props.instance.status)}
              onClick={() =>
                props.transitionRun(props.instance.id, props.instance.status === "paused" ? "resume" : "pause")
              }
            >
              <PauseCircle size={16} />
              {props.instance.status === "paused" ? "Resume" : "Pause"}
            </button>
            <button
              class="danger-button"
              disabled={isTerminal(props.instance.status)}
              onClick={() => props.transitionRun(props.instance.id, "cancel")}
            >
              <XCircle size={16} />
              Kill
            </button>
          </div>
        </div>
      </aside>
    </div>
  );
}

function RunActions(props: {
  instance: ScriptInstance;
  transitionRun: (id: string, action: "pause" | "resume" | "cancel") => void;
  compact?: boolean;
}) {
  const terminal = () => isTerminal(props.instance.status);
  const pauseAction = () => (props.instance.status === "paused" ? "resume" : "pause");
  const pauseLabel = () => (props.instance.status === "paused" ? "Resume" : "Pause");
  const stop = (event: MouseEvent) => event.stopPropagation();

  return (
    <div class={`run-actions ${props.compact ? "compact" : ""}`}>
      <button
        class="soft-button"
        disabled={terminal()}
        onClick={(event) => {
          stop(event);
          props.transitionRun(props.instance.id, pauseAction());
        }}
      >
        <PauseCircle size={16} />
        {pauseLabel()}
      </button>
      <button
        class="danger-button"
        disabled={terminal()}
        onClick={(event) => {
          stop(event);
          props.transitionRun(props.instance.id, "cancel");
        }}
      >
        <XCircle size={16} />
        Kill
      </button>
    </div>
  );
}

function WorkflowGraph(props: {
  instance: ScriptInstance;
  selectedNodeId: string;
  setSelectedNodeId: (id: string) => void;
}) {
  const nodeById = createMemo(() => new Map(props.instance.graph.nodes.map((node) => [node.id, node])));
  return (
    <section class="graph-panel">
      <div class="graph-toolbar">
        <div>
          <h2>Definition Graph With Live Status</h2>
          <p>Rules rewrite facts and materialize durable effects.</p>
        </div>
        <div class="legend">
          <LegendItem label="completed" status="completed" />
          <LegendItem label="queued" status="queued" />
          <LegendItem label="running" status="running" />
          <LegendItem label="blocked" status="blocked" />
          <LegendItem label="cancelled" status="cancelled" />
          <LegendItem label="not reached" status="skipped" />
        </div>
      </div>
      <div class="graph-canvas">
        <svg class="edge-layer" viewBox="0 0 820 360" preserveAspectRatio="none" aria-hidden="true">
          <For each={props.instance.graph.edges}>
            {(edge) => {
              const from = nodeById().get(edge.from)!;
              const to = nodeById().get(edge.to)!;
              const nodeWidth = 116;
              const x1 = from.x + (to.x > from.x ? nodeWidth : nodeWidth / 2);
              const y1 = from.y + 34;
              const x2 = to.x + (to.x > from.x ? 0 : nodeWidth / 2);
              const y2 = to.y + 34;
              const mid = x1 + (x2 - x1) / 2;
              return (
                <>
                  <path
                    class={`graph-edge ${edge.status}`}
                    d={`M ${x1} ${y1} C ${mid} ${y1}, ${mid} ${y2}, ${x2} ${y2}`}
                  />
                  <title>{edge.label}</title>
                </>
              );
            }}
          </For>
        </svg>
        <For each={props.instance.graph.nodes}>
          {(node) => (
            <button
              class={`graph-node ${props.selectedNodeId === node.id ? "selected" : ""} ${node.kind} ${node.status}`}
              style={{ left: `${node.x}px`, top: `${node.y}px` }}
              onClick={() => props.setSelectedNodeId(node.id)}
            >
              <NodeIcon kind={node.kind} />
              <span>
                <strong>{node.label}</strong>
                <small>{node.detail}</small>
              </span>
            </button>
          )}
        </For>
      </div>
    </section>
  );
}

function EventTimeline(props: { instance: ScriptInstance }) {
  return (
    <section class="timeline-panel">
      <div class="panel-heading compact">
        <h2>Event Timeline</h2>
        <Route size={18} />
      </div>
      <div class="timeline">
        <For each={props.instance.events}>
          {(event) => (
            <div class={`timeline-item ${event.status}`}>
              <time>{event.time}</time>
              <div>
                <strong>{event.type}</strong>
                <span>{event.summary}</span>
              </div>
            </div>
          )}
        </For>
      </div>
    </section>
  );
}

function ApiKeys() {
  return (
    <section class="panel full-page">
      <div class="panel-heading">
        <div>
          <h1>API Keys</h1>
          <p>Provider credentials and capability bindings for workflow effects.</p>
        </div>
        <button class="primary-button">
          <KeyRound size={16} />
          Add key
        </button>
      </div>
      <div class="key-table">
        <For each={apiKeys}>
          {(key) => (
            <div class="key-row">
              <div class="key-name">
                <KeyRound size={18} />
                <div>
                  <strong>{key.name}</strong>
                  <span>{key.scope}</span>
                </div>
              </div>
              <span class={`key-status ${key.status}`}>{key.status}</span>
              <span>{key.lastUsed}</span>
              <button class="soft-button">Rotate</button>
            </div>
          )}
        </For>
      </div>
    </section>
  );
}

function SettingsView() {
  return (
    <section class="panel full-page">
      <div class="panel-heading">
        <div>
          <h1>Settings</h1>
          <p>Prototype controls for runtime inspection and provider safety.</p>
        </div>
      </div>
      <div class="settings-grid">
        <Setting title="Fixture-first validation" detail="Run deterministic providers before native adapters." enabled />
        <Setting title="Require human approval for revise" detail="Keep workflow revision as a control-plane action." enabled />
        <Setting title="Live provider writes" detail="Allow repo-writing providers to modify workspaces." />
        <Setting title="Trace conformance checks" detail="Run trace reconstruction after terminal workflow states." enabled />
      </div>
    </section>
  );
}

function Metric(props: { icon: IconComponent; label: string; value: string; tone: string }) {
  const Icon = props.icon;
  return (
    <div class={`metric ${props.tone}`}>
      <Icon size={20} />
      <div>
        <span>{props.label}</span>
        <strong>{props.value}</strong>
      </div>
    </div>
  );
}

function Counter(props: { label: string; value: number }) {
  return (
    <span class="counter">
      <strong>{props.value}</strong>
      {props.label}
    </span>
  );
}

function Setting(props: { title: string; detail: string; enabled?: boolean }) {
  return (
    <label class="setting">
      <input type="checkbox" checked={Boolean(props.enabled)} />
      <span>
        <strong>{props.title}</strong>
        <small>{props.detail}</small>
      </span>
    </label>
  );
}

function LegendItem(props: { label: string; status: NodeStatus }) {
  return (
    <span class="legend-item">
      <i class={props.status} />
      {props.label}
    </span>
  );
}

function NodeIcon(props: { kind: NodeKind }) {
  if (props.kind === "fact") return <Database size={18} />;
  if (props.kind === "rule") return <GitBranch size={18} />;
  if (props.kind === "effect") return <Server size={18} />;
  if (props.kind === "terminal") return <CheckCircle2 size={18} />;
  return <Boxes size={18} />;
}

function StatusPill(props: { status: InstanceStatus }) {
  return <span class={`status-pill ${props.status}`}>{props.status}</span>;
}

function StatusBadge(props: { status: NodeStatus }) {
  const icon = () => {
    if (props.status === "completed" || props.status === "fired" || props.status === "active") return <CheckCircle2 size={16} />;
    if (props.status === "failed" || props.status === "cancelled") return <XCircle size={16} />;
    if (props.status === "queued" || props.status === "blocked") return <Clock3 size={16} />;
    return <Activity size={16} />;
  };
  return (
    <span class={`status-badge ${props.status}`}>
      {icon()}
      {props.status}
    </span>
  );
}

render(() => <App />, document.getElementById("root")!);
