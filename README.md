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
| **AI Chat (`:chat`)** | Ask questions about your cluster — OpenAI-compatible, Google Antigravity (ADC), AWS Bedrock, Azure OpenAI, or Ollama |
| **Multi-provider AI** | Five providers: `api` (OpenAI-compatible), `antigravity` (Vertex AI/ADC), `bedrock` (AWS), `azure`, `ollama` (local) |
| **Chat history (`:chats`)** | Sessions auto-saved to disk; browse and restore any of the last 50 conversations; export to Markdown with `Ctrl+S` / `E` |
| **Security-first** | Sanitizer layer strips secrets, tokens, and passwords before any data reaches the LLM |
| **Log analysis** | Smart log compression — 10K lines → ~200 tokens of signal |
| **Port-forward manager (`:pf`)** | Start, list, and kill `kubectl port-forward` sessions from the TUI |
| **Expert mode (`:expert`)** | Automated AI scan for pod failures, log spam, and performance issues; one-click remediation playbooks |
| **Helm manager (`:helm`)** | Browse, inspect, delete, and roll back Helm releases; sanitized values and manifest views |
| **KubeVela support (`:vela`)** | Inspect OAM Applications, component health trees, workflow steps, revision history, and definitions — no `vela` binary needed for reads |
| **Cluster metadata store (`:meta`)** | Local journal of cluster snapshots, issues, and operator actions; history injected automatically into every AI prompt |
| **Animated logo** | Cross-dissolve header logo that cycles through visual styles |
| **Shell exec** | `kubectl exec -it` into pods without leaving the UI |
| **Plugins** | Extend with custom shell commands bound to any resource type |

---

## Installation

### Pre-built Binaries (Recommended)

Download the archive for your platform from the [GitHub Releases](https://github.com/neuro-ng/k7s/releases) page. Extract the archive and move the `k7s` binary to your `$PATH`.

**Linux (x86_64 musl)**
```bash
VERSION="v0.1.0" # Replace with the latest version
curl -L "https://github.com/neuro-ng/k7s/releases/download/${VERSION}/k7s-${VERSION}-x86_64-unknown-linux-musl.tar.gz" | tar xz
sudo mv k7s /usr/local/bin/
```

**macOS (Apple Silicon)**
```bash
VERSION="v0.1.0" # Replace with the latest version
curl -L "https://github.com/neuro-ng/k7s/releases/download/${VERSION}/k7s-${VERSION}-aarch64-apple-darwin.tar.gz" | tar xz
sudo mv k7s /usr/local/bin/
```

### Shell Completions

The release archives include a `completions/` directory with shell scripts for `bash`, `zsh`, `fish`, and `PowerShell`. To enable them, copy the appropriate script to your shell's completions directory.

**Zsh:**
```bash
# Example for zsh (using oh-my-zsh)
mkdir -p ~/.oh-my-zsh/completions
cp completions/_k7s ~/.oh-my-zsh/completions/
# Restart your shell or run `source ~/.zshrc`
```

**Bash:**
```bash
# Example for bash
sudo cp completions/k7s.bash /etc/bash_completion.d/k7s
# Restart your shell or run `source ~/.bashrc`
```

### From source

```bash
# Prerequisites: Rust 1.77+, kubectl in $PATH
git clone https://github.com/neuro-ng/k7s
cd k7s
cargo install --path .
```

### Docker

Two image variants are published to `ghcr.io/neuro-ng/k7s`:

| Tag | Contents | Use case |
|-----|----------|----------|
| `:latest` / `:<version>` | k7s binary only (distroless) | Minimal read-only cluster inspection |
| `:full` / `:<version>-full` | k7s + kubectl + helm + vela + trivy + gcloud | Full feature set, zero local dependencies |

**Minimal image** (read-only cluster browsing):

```bash
docker run --rm -it \
  -v "$HOME/.kube:/home/k7s/.kube:ro" \
  ghcr.io/neuro-ng/k7s:latest
```

**Batteries-included image** (all features):

```bash
docker run --rm -it \
  -v "$HOME/.kube:/home/k7s/.kube:ro" \
  -v "$HOME/.config/k7s:/home/k7s/.config/k7s" \
  -v "$HOME/.local/state/k7s:/home/k7s/.local/state/k7s" \
  ghcr.io/neuro-ng/k7s:full
```

With AWS Bedrock AI provider:

```bash
docker run --rm -it \
  -v "$HOME/.kube:/home/k7s/.kube:ro" \
  -e AWS_ACCESS_KEY_ID \
  -e AWS_SECRET_ACCESS_KEY \
  -e AWS_REGION \
  ghcr.io/neuro-ng/k7s:full
```

With Google Antigravity ADC:

```bash
docker run --rm -it \
  -v "$HOME/.kube:/home/k7s/.kube:ro" \
  -v "$HOME/.config/gcloud:/home/k7s/.config/gcloud:ro" \
  -e GOOGLE_APPLICATION_CREDENTIALS=/home/k7s/.config/gcloud/application_default_credentials.json \
  ghcr.io/neuro-ng/k7s:full
```

**Docker Compose** (persistent state across sessions):

```bash
docker compose -f packaging/docker-compose.yml run --rm k7s
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
| `:helm` / `:hr` | Helm release browser |
| `:vela` / `:va` | KubeVela application browser |
| `:veladefs` / `:vd` | KubeVela definition catalog |
| `:meta` / `:cmeta` | Cluster metadata journal browser |
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

## Helm Manager

Open `:helm` (alias `:hr`) to browse all Helm releases across namespaces.

| Key | Action |
|-----|--------|
| `Enter` | Revision history for selected release |
| `v` | Sanitized values for selected release/revision |
| `m` | Rendered manifest for selected release/revision |
| `n` | NOTES.txt for selected release/revision |
| `D` | Uninstall (confirm dialog) |
| `r` | Rollback to selected revision (in history view) |
| `A` | Inject release context into AI chat |

Read operations (list, history, values) query Kubernetes Secrets directly — no `helm` binary required. Rollback and uninstall delegate to the `helm` binary.

---

## KubeVela Support

Open `:vela` (alias `:va`) to browse KubeVela Applications. k7s reads all OAM resources directly from the Kubernetes API — no `vela` binary required for inspection.

| Key | Action |
|-----|--------|
| `Enter` | Component + trait health tree |
| `w` | Workflow step view |
| `h` | Revision history |
| `r` | Refresh list |
| `A` | Inject sanitized app context into AI chat |

Open `:veladefs` (alias `:vd`) to browse ComponentDefinitions, TraitDefinitions, and WorkflowStepDefinitions. Press `Tab` to cycle definition types.

If KubeVela is not installed on the cluster, k7s shows an actionable hint instead of an error.

---

## Cluster Metadata Store

k7s silently journals every session to `~/.local/state/k7s/cluster-metadata/<context>/` as append-only daily JSON-lines files. Three record types are captured automatically:

- **Snapshot** — node/workload counts written on each cluster connect
- **Issue** — expert-scan alerts (CrashLoopBackOff, OOMKilled, etc.) with optional `resolved_at`
- **Interaction** — operator actions (delete, scale, port-forward, AI analyse, etc.)

The last 7 days of history (configurable) are injected as a ~300-token summary block into every AI prompt, giving the LLM awareness of recurring issues, operator intent, and cluster drift.

Open `:meta` (alias `:cmeta`) to browse the journal:

| Key | Action |
|-----|--------|
| `Enter` | Full JSON detail for selected record |
| `f` | Cycle filter (All / Snapshot / Issue / Interaction) |
| `c` | Inject selected record into AI chat |
| `p` | Prune old records |
| `r` | Refresh |

CLI access:
```bash
k7s meta list   [--days N] [--type snapshot|issue|interaction]
k7s meta show   <date>
k7s meta prune  [--before <date>]
k7s meta export [--output json|markdown]
```

---

## AI Chat

k7s includes a built-in AI assistant that analyses your cluster without exposing secrets.

### Providers

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

**Option B — Google Antigravity (ADC / Vertex AI)**

```bash
gcloud auth application-default login
```

`~/.config/k7s/config.yaml`:
```yaml
k7s:
  ai:
    provider: antigravity
    gcpProject: my-project-id   # optional; detected from ADC if omitted
    gcpRegion: us-central1
    model: gemini-2.0-flash-001
```

**Option C — AWS Bedrock**

```bash
export AWS_ACCESS_KEY_ID="..."
export AWS_SECRET_ACCESS_KEY="..."
export AWS_REGION="us-east-1"
```

`~/.config/k7s/config.yaml`:
```yaml
k7s:
  ai:
    provider: bedrock
    model: anthropic.claude-sonnet-4-5
```

Standard AWS credential chain is supported (env vars, `~/.aws/credentials`, instance profiles).

**Option D — Azure OpenAI**

```bash
export AZURE_OPENAI_API_KEY="..."
export AZURE_OPENAI_ENDPOINT="https://<resource>.openai.azure.com/openai/deployments/<deployment>"
```

`~/.config/k7s/config.yaml`:
```yaml
k7s:
  ai:
    provider: azure
    model: gpt-4o
```

**Option E — Ollama (local)**

```bash
# Start Ollama with your preferred model
ollama serve
ollama pull llama3
```

`~/.config/k7s/config.yaml`:
```yaml
k7s:
  ai:
    provider: ollama
    endpoint: http://localhost:11434   # default; override with OLLAMA_HOST
    model: llama3
```

No API key or cloud account required. Supports any model available in your Ollama installation.

### Capabilities

| Capability | How |
|-----------|-----|
| Error analysis | `:chat` → ask about a failing pod |
| Log troubleshooting | `l` on a pod, then `A` to add to chat |
| Context injection | `A` on any resource injects sanitized metadata + events |
| Helm context | `A` on a Helm release injects sanitized chart context |
| Vela context | `A` on a KubeVela app injects sanitized OAM context |
| Historical context | Cluster metadata journal auto-injected into every prompt |
| Efficiency review | `:chat` → ask about resource sizing |
| Cluster health | `:chat` → ask about overall health |
| Automated scan | `:expert` for AI-driven anomaly detection |

### Chat history

Sessions are saved automatically to `~/.local/state/k7s/chat_logs/`. The most recent session is restored on startup. Browse all sessions with `:chats` (up to 50 kept). Export any session to Markdown with `Ctrl+S` in chat or `E` in `:chats`.

### Security guarantee

The sanitizer **always** runs before any data reaches the LLM:

- All `v1/Secret` data fields are stripped
- Environment variable *values* are stripped (names kept)
- ConfigMap *values* are stripped (keys kept)
- Helm values matching secret key patterns (`password`, `token`, `key`, `url`, etc.) are redacted
- KubeVela component properties with `env`, `secretRef`, `configRef` are sanitized
- Values matching secret patterns (JWT, connection strings, API keys) are redacted
- Logs are compressed — raw streams never leave the process
- Cluster metadata records are sanitized before disk write

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
  helm:
    enabled: true
    defaultNamespace: ""          # empty = all namespaces
    maxHistory: 10
  vela:
    enabled: true
    defaultNamespace: ""
    velaPath: vela                # path to vela binary (default: resolve from PATH)
  meta:
    enabled: true
    retentionDays: 90
    maxSizeBytes: 52428800        # 50 MB
    snapshotOnConnect: true
    recordInteractions: true
    historyDaysForContext: 7      # days of history injected into LLM context
```

### Environment variables

| Variable | Description |
|----------|-------------|
| `K7S_LLM_API_KEY` | API key for OpenAI-compatible provider |
| `K7S_CONFIG_DIR` | Override config directory (default: XDG) |
| `K7S_LOGS_DIR` | Override log directory |
| `K7S_SANITIZER_STRICT` | Force strict sanitizer mode (`true`/`false`) |
| `GOOGLE_APPLICATION_CREDENTIALS` | Path to ADC JSON (Antigravity) |
| `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_REGION` | AWS credentials (Bedrock) |
| `AWS_PROFILE` | AWS named profile (Bedrock) |
| `AZURE_OPENAI_API_KEY` | API key for Azure OpenAI |
| `AZURE_OPENAI_ENDPOINT` | Azure OpenAI deployment endpoint URL |
| `OLLAMA_HOST` | Ollama server URL (default: `http://localhost:11434`) |
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
