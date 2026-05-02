# k7s

**Performance-focused, security-first Kubernetes TUI with AI-powered cluster analysis.**

k7s is a clean-room Rust reimplementation of the [k9s](https://github.com/derailed/k9s) concept,
extended with a built-in AI chat window and a sanitizer layer that guarantees secrets never reach
any LLM.

---

## Features

| Feature | Description |
|---------|-------------|
| **Fast TUI** | < 200 ms startup, < 50 ms refresh latency |
| **Full resource coverage** | Pods, Deployments, StatefulSets, DaemonSets, Services, HPAs, PDBs, LimitRanges, ResourceQuotas, EndpointSlices, StorageClasses, RBAC, CRDs, and more |
| **AI Chat (`:chat`)** | Ask questions about your cluster — OpenAI-compatible API or Google Antigravity (ADC) |
| **Chat history (`:chats`)** | Sessions auto-saved to disk; browse and restore any of the last 50 conversations |
| **Security-first** | Sanitizer layer strips secrets, tokens, and passwords before any data reaches the LLM |
| **Log analysis** | Smart log compression — 10K lines → ~200 tokens of signal |
| **Port-forward manager (`:pf`)** | Start, list, and kill `kubectl port-forward` sessions from the TUI |
| **Expert mode (`:expert`)** | Automated AI scan for pod failures, log spam, and performance issues |
| **Animated logo** | Cross-dissolve header logo that cycles through visual styles |
| **Shell exec** | `kubectl exec -it` into pods without leaving the UI |
| **Helm view** | Browse, delete, and roll back Helm releases |
| **Plugins** | Extend with custom shell commands bound to any resource type |

---

## Installation

### From source

```bash
# Prerequisites: Rust 1.77+, kubectl in $PATH
git clone https://github.com/neuro-ng/k7s
cd k7s
cargo install --path .
```

### Docker

```bash
docker run --rm -it \
  -v "$HOME/.kube:/root/.kube:ro" \
  -v "$HOME/.config/k7s:/root/.config/k7s:ro" \
  ghcr.io/neuro-ng/k7s:latest
```

---

## Quick Start

```bash
# Open the TUI (uses active kubeconfig context)
k7s

# Connect to a specific context
k7s --context my-cluster

# Watch a specific namespace
k7s --namespace kube-system

# Read-only mode (no mutations)
k7s --readonly
```

---

## Resource Views

Type `:` to open the command prompt (Tab-completion included).

### Workloads
`:po` · `:dp` · `:sts` · `:ds` · `:rs` · `:job` · `:cj`

### Networking
`:svc` · `:ep` · `:eps` · `:ing` · `:netpol`

### Config & Storage
`:cm` · `:secret` · `:pv` · `:pvc` · `:sc`

### Policy & Access
`:hpa` · `:pdb` · `:lr` · `:rq` · `:role` · `:rb` · `:cr` · `:crb` · `:sa` · `:crd`

### Cluster
`:no` · `:ns` · `:ev`

### Special Views

| Command | Description |
|---------|-------------|
| `:ctx` | Switch kubeconfig context |
| `:pulse` | Cluster health dashboard |
| `:wl` | Aggregated workload overview |
| `:xray` | Resource dependency tree |
| `:metrics` / `:top` | Live CPU/memory sparklines |
| `:expert` | AI-driven anomaly detection and remediation |
| `:pf` | Active port-forward manager |
| `:chat` | AI chat window |
| `:chats` | Browse persisted AI chat sessions |
| `:alias` | All registered resource aliases |
| `:dir` | Local filesystem browser |

---

## Key Bindings

| Key | Action |
|-----|--------|
| `d` | Describe selected resource |
| `y` | View YAML |
| `l` | Stream logs (pods) |
| `s` | Shell into pod / Scale workload |
| `r` | Restart workload |
| `t` | Trigger CronJob |
| `f` | Port-forward to selected pod |
| `A` | Inject resource into AI chat as context |
| `c` / `u` | Cordon / Uncordon node |
| `D` | Delete resource (confirm dialog) — kill port-forward in `:pf` |
| `/` | Filter rows |
| `Enter` | Drill down (pod → containers, etc.) |
| `q` | Quit / go back |
| `?` | Help overlay |

---

## Port-Forward Manager

Press `f` on any pod to start a port-forward, or annotate resources for automatic setup:

```yaml
metadata:
  annotations:
    k7s.io/portforward: "9090:8080,5432:5432"
```

Open `:pf` to list all active forwards. Press `D` on any row to kill it.

---

## AI Chat

k7s includes a built-in AI assistant that analyses your cluster without exposing secrets.

### Setup

**Option A — API key (OpenAI-compatible)**

```bash
export K7S_LLM_API_KEY="sk-..."
```

`~/.config/k7s/config.yaml`:
```yaml
k7s:
  ai:
    provider: api
    endpoint: https://api.openai.com/v1/chat/completions
    model: gpt-4o
```

**Option B — Google Antigravity (ADC)**

```bash
gcloud auth application-default login
```

`~/.config/k7s/config.yaml`:
```yaml
k7s:
  ai:
    provider: antigravity
```

### Capabilities

| Capability | How |
|-----------|-----|
| Error analysis | `:chat` → ask about a failing pod |
| Log troubleshooting | `l` on a pod, then `A` to add to chat |
| Context injection | `A` on any resource injects sanitized metadata + events |
| Efficiency review | `:chat` → ask about resource sizing |
| Cluster health | `:chat` → ask about overall health |
| Automated scan | `:expert` for AI-driven anomaly detection |

### Chat history

Sessions are saved automatically to `~/.local/state/k7s/chat_logs/`. The most recent session is restored on startup. Browse all sessions with `:chats` (up to 50 kept).

### Security guarantee

The sanitizer **always** runs before any data reaches the LLM:

- All `v1/Secret` data fields are stripped
- Environment variable *values* are stripped (names kept)
- ConfigMap *values* are stripped (keys kept)
- Values matching secret patterns (JWT, connection strings, API keys) are redacted
- Logs are compressed — raw streams never leave the process

---

## Configuration

`~/.config/k7s/config.yaml`:

```yaml
k7s:
  refreshRate: 2
  readOnly: false
  ui:
    skin: dracula                 # default, dracula, monokai, or custom YAML
    enableMouse: false
    logoTransitionSpeed: 3        # logo animation speed 1–10
  logger:
    tail: 200
    buffer: 5000
  ai:
    provider: api
    tokenBudget:
      maxPerSession: 100000
      maxPerQuery: 4000
      warnAt: 80000
    sanitizer:
      strictMode: true
      auditLog: true
      customPatterns:
        - "(?i)my-secret-prefix\\s*[:=]\\s*\\S+"
```

### Environment variables

| Variable | Description |
|----------|-------------|
| `K7S_LLM_API_KEY` | API key for LLM provider |
| `K7S_CONFIG_DIR` | Override config directory (default: XDG) |
| `K7S_LOGS_DIR` | Override log directory |
| `K7S_SANITIZER_STRICT` | Force strict sanitizer mode (`true`/`false`) |
| `GOOGLE_APPLICATION_CREDENTIALS` | Path to ADC JSON (Antigravity) |
| `KUBECONFIG` | Kubeconfig path (standard kubectl env) |

---

## Plugins

`~/.config/k7s/plugins.yaml`:

```yaml
kubectl-debug:
  shortCut: Ctrl-x
  description: Debug pod
  scopes: [pods]
  command: kubectl
  args: ["debug", "-it", "$NAME", "-n", "$NAMESPACE", "--image=busybox"]
  background: false
  confirm: false
```

Variables: `$NAME`, `$NAMESPACE`, `$CONTEXT`, `$CLUSTER`

---

## Performance

| Metric | Target | Status |
|--------|--------|--------|
| Startup to first render | < 200 ms | ✅ |
| Memory (idle, 1 cluster) | < 15 MB | ✅ |
| Memory (active, 1000+ resources) | < 50 MB | ✅ |
| Screen refresh latency | < 50 ms | ✅ |
| Log sanitization throughput | > 50K lines/sec | ✅ |

---

## Building & Testing

```bash
cargo build                          # debug
cargo build --release                # optimized
cargo test                           # all tests
cargo test sanitizer                 # sanitizer tests (critical path)
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo bench
```

---

## License

Apache-2.0 — see [LICENSE](LICENSE).
