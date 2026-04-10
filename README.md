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
| **All k9s resources** | Pods, Deployments, StatefulSets, DaemonSets, Services, Nodes, and more |
| **AI Chat (`:chat`)** | Ask questions about your cluster — powered by any OpenAI-compatible API or Google Antigravity (ADC) |
| **Security-first** | Sanitizer layer strips secrets, tokens, and passwords before any data reaches the LLM |
| **Log analysis** | Smart log compression — 10K lines → ~200 tokens of signal |
| **Port-forwarding** | Manage `kubectl port-forward` sessions from the TUI |
| **Shell exec** | `kubectl exec -it` into pods without leaving the UI |
| **Helm view** | Browse, delete, and roll back Helm releases |
| **Plugins** | Extend with custom shell commands bound to any resource type |
| **Themes** | Dracula, Monokai, and custom YAML skins |

---

## Installation

### From source

```bash
# Prerequisites: Rust 1.77+, kubectl in $PATH
git clone https://github.com/your-org/k7s
cd k7s
cargo install --path .
```

### Docker

```bash
docker run --rm -it \
  -v "$HOME/.kube:/root/.kube:ro" \
  -v "$HOME/.config/k7s:/root/.config/k7s:ro" \
  ghcr.io/your-org/k7s:latest
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

# Headless (print config and exit)
k7s --headless
```

---

## Key Bindings

| Key | Action |
|-----|--------|
| `:pod` | Switch to Pod view |
| `:deploy` | Switch to Deployment view |
| `:svc` | Switch to Service view |
| `:node` | Switch to Node view |
| `:ns` | Switch to Namespace view |
| `:helm` | Switch to Helm release view |
| `:chat` | Open AI chat window |
| `Enter` | Describe selected resource |
| `l` | Stream logs (pods) |
| `s` | Shell exec (pods) / Scale (deployments) |
| `d` | Describe resource |
| `y` | View YAML |
| `Ctrl-d` | Delete resource |
| `r` | Restart workload |
| `t` | Trigger CronJob |
| `c` | Cordon node |
| `u` | Uncordon node |
| `/` | Filter rows |
| `q` | Quit / close panel |
| `?` | Help |

---

## AI Chat

k7s includes a built-in AI assistant that can analyse your cluster without exposing secrets.

### Setup

**Option A — API key (OpenAI-compatible)**

```bash
export K7S_LLM_API_KEY="sk-..."
# Then in ~/.config/k7s/config.yaml:
# ai:
#   provider: api
#   endpoint: https://api.openai.com/v1/chat/completions
#   model: gpt-4o
```

**Option B — Google Antigravity (ADC)**

```bash
gcloud auth application-default login
# In config.yaml:
# ai:
#   provider: antigravity
```

### Capabilities

| Capability | Command | What it sends |
|-----------|---------|---------------|
| Error analysis | `:chat` → ask about a failing pod | Pod metadata + events (sanitized) |
| Log troubleshooting | `l` on a pod → `a` for AI | Compressed log summary |
| Efficiency review | `:chat` → efficiency | Resource requests/limits across workloads |
| Cluster health | `:chat` → health | Node conditions + recent events |
| RBAC audit | `:chat` → rbac | Role/binding structure (no tokens) |

### Security guarantee

The sanitizer layer **always** runs before any data reaches the LLM:

- All `v1/Secret` data fields are stripped
- Environment variable *values* are stripped (names are kept)
- ConfigMap *values* are stripped (keys are kept)
- Any value matching a secret pattern (JWT, connection string, API key regex) is redacted
- Logs are compressed and deduplicated — raw log streams never leave the cluster

---

## Configuration

Config file: `~/.config/k7s/config.yaml`

```yaml
k7s:
  refreshRate: 2           # seconds between resource list refreshes
  readOnly: false          # disable all mutating operations
  ui:
    skin: dracula          # built-in: default, dracula, monokai; or custom YAML
    enableMouse: false
  logger:
    tail: 200              # lines to tail on log open
    buffer: 5000           # ring buffer size
  ai:
    provider: api          # "api" or "antigravity"
    tokenBudget:
      maxPerSession: 100000
      maxPerQuery: 4000
      warnAt: 80000
    sanitizer:
      strictMode: true     # default-deny (recommended)
      auditLog: true
      customPatterns:
        - "(?i)my-internal-secret-prefix\\s*[:=]\\s*\\S+"
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

Add custom actions to any resource view via `~/.config/k7s/plugins.yaml`:

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

## Skins

Custom skin at `~/.config/k7s/skins/my-theme.yaml`:

```yaml
general:
  fg: "#cdd6f4"
  bg: "#1e1e2e"
header:
  fg: "#89b4fa"
  bg: "#1e1e2e"
table_header:
  fg: "#a6e3a1"
  bg: "#1e1e2e"
selected_row:
  fg: "#1e1e2e"
  bg: "#89b4fa"
```

---

## Performance

| Metric | Target | Status |
|--------|--------|--------|
| Startup to first render | < 200 ms | ✅ |
| Memory (idle, 1 cluster) | < 15 MB | ✅ |
| Memory (active, 1000+ resources) | < 50 MB | ✅ |
| Screen refresh latency | < 50 ms | ✅ |
| Log sanitization throughput | > 50K lines/sec | ✅ |

Run benchmarks: `cargo bench`

---

## Building & Testing

```bash
# Debug build
cargo build

# Release build (optimized, stripped)
cargo build --release

# Tests
cargo test

# Sanitizer tests (critical path — run these first)
cargo test sanitizer

# Benchmarks
cargo bench

# Lint
cargo clippy -- -D warnings

# Format check
cargo fmt --check
```

---

## License

Apache-2.0 — see [LICENSE](LICENSE).
