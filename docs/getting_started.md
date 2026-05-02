# Getting Started with k7s

Welcome to **k7s**, a high-performance, security-first Kubernetes TUI (Terminal UI) with built-in AI capabilities.

This guide will walk you through the essential steps to get `k7s` up and running, and teach you how to navigate its interface effectively.

## Prerequisites

- **Rust** 1.77+
- **kubectl** available in your `$PATH`
- A valid `kubeconfig` file (usually at `~/.kube/config`) pointing to a Kubernetes cluster

## 1. Installation

### Building from Source

```bash
git clone https://github.com/neuro-ng/k7s
cd k7s
cargo install --path .
```

### Running with Docker

```bash
docker run --rm -it \
  -v "$HOME/.kube:/root/.kube:ro" \
  -v "$HOME/.config/k7s:/root/.config/k7s:ro" \
  ghcr.io/neuro-ng/k7s:latest
```

## 2. Launching k7s

```bash
# Active kubeconfig context
k7s

# Specific context
k7s --context my-cluster

# Filter to a namespace
k7s --namespace kube-system

# Read-only mode (prevents accidental mutations)
k7s --readonly

# Debug logging
k7s -l debug
```

## 3. The Interface

The header shows your cluster context and connection status on the left, and an animated k7s logo on the right that cycles through visual styles. The footer displays available key actions for the active view.

Type `:` to open the command prompt (autocomplete included), or `?` for the help overlay.

## 4. Navigating Resources

Switch views with colon commands. All aliases support Tab-completion in the prompt.

### Workloads

| Command | Resource |
|---------|----------|
| `:po` / `:pod` | Pods |
| `:dp` / `:deploy` | Deployments |
| `:sts` / `:statefulset` | StatefulSets |
| `:ds` / `:daemonset` | DaemonSets |
| `:rs` / `:replicaset` | ReplicaSets |
| `:job` | Jobs |
| `:cj` / `:cronjob` | CronJobs |

### Networking

| Command | Resource |
|---------|----------|
| `:svc` / `:service` | Services |
| `:ep` / `:endpoints` | Endpoints |
| `:eps` / `:endpointslice` | EndpointSlices |
| `:ing` / `:ingress` | Ingresses |
| `:netpol` / `:networkpolicy` | NetworkPolicies |

### Config & Storage

| Command | Resource |
|---------|----------|
| `:cm` / `:configmap` | ConfigMaps |
| `:secret` | Secrets |
| `:pv` | PersistentVolumes |
| `:pvc` | PersistentVolumeClaims |
| `:sc` / `:storageclass` | StorageClasses |

### Policy & Access

| Command | Resource |
|---------|----------|
| `:hpa` | HorizontalPodAutoscalers |
| `:pdb` | PodDisruptionBudgets |
| `:lr` / `:limitrange` | LimitRanges |
| `:rq` / `:resourcequota` | ResourceQuotas |
| `:role` | Roles |
| `:rb` / `:rolebinding` | RoleBindings |
| `:cr` / `:clusterrole` | ClusterRoles |
| `:crb` / `:clusterrolebinding` | ClusterRoleBindings |
| `:sa` / `:serviceaccount` | ServiceAccounts |
| `:crd` | CustomResourceDefinitions |

### Cluster

| Command | Resource |
|---------|----------|
| `:no` / `:node` | Nodes |
| `:ns` / `:namespace` | Namespaces |
| `:ev` / `:event` | Events |

### Special Views

| Command | Description |
|---------|-------------|
| `:ctx` / `:context` | Switch kubeconfig context |
| `:alias` / `:aliases` | Browse all known resource aliases |
| `:pulse` | Cluster health dashboard |
| `:wl` / `:workload` | Aggregated workload overview |
| `:xray` | Resource dependency tree |
| `:metrics` / `:top` | Live CPU/memory sparklines |
| `:expert` | AI-driven anomaly detection |
| `:dir` | Local filesystem browser |
| `:pf` / `:portforwards` | Active port-forward manager |
| `:chat` | AI chat window |
| `:chats` / `:chat-history` | Browse persisted AI chat sessions |

## 5. Key Actions

Actions appear in the footer and depend on the selected resource type.

| Key | Action |
|-----|--------|
| `d` | Describe selected resource |
| `y` | View YAML |
| `l` | Stream logs (Pods) |
| `s` | Shell into pod / Scale workload |
| `r` | Restart workload |
| `t` | Trigger CronJob manually |
| `c` / `u` | Cordon / Uncordon node |
| `f` | Port-forward to selected pod |
| `D` | Delete resource (with confirmation) — or kill port-forward in `:pf` |
| `A` | Inject selected resource into AI chat as context |
| `/` | Filter rows |
| `Enter` | Drill down (pods → containers, etc.) |
| `q` | Go back / quit |
| `?` | Help overlay |

## 6. Port-Forward Manager

Start a port-forward from any pod view by pressing `f`, or launch them automatically via the `k7s.io/portforward` annotation:

```yaml
metadata:
  annotations:
    k7s.io/portforward: "9090:8080,5432:5432"
```

Open `:pf` to see all active forwards. Press `D` on any row to kill it.

## 7. AI Assistant

k7s has a built-in AI assistant. All cluster data is sanitized before it leaves the process — secrets, tokens, and raw config values are never sent to the LLM.

### OpenAI-compatible API

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

### Google Antigravity (ADC)

```bash
gcloud auth application-default login
```

`~/.config/k7s/config.yaml`:

```yaml
k7s:
  ai:
    provider: antigravity
```

### Chat commands

- **`:chat`** — Open the AI chat window. Ask questions about your cluster ("Why is this pod crash-looping?").
- **`A`** — While browsing any resource, press `A` to inject its sanitized metadata, events, and log summary as context into the current chat session.
- **`:chats`** — Browse previously saved chat sessions. Sessions are saved automatically and the most recent one is restored on startup (up to 50 kept).

## 8. Configuration

`~/.config/k7s/config.yaml` — selected options:

```yaml
k7s:
  refreshRate: 2          # seconds between auto-refresh
  readOnly: false
  ui:
    enableMouse: false
    logoTransitionSpeed: 3   # logo animation speed 1–10 (default 3)
  ai:
    provider: api
    tokenBudget:
      maxPerSession: 100000
      maxPerQuery: 4000
      warnAt: 80000
    sanitizer:
      strictMode: true
      auditLog: true
```

Custom redaction patterns can be added under `sanitizer.customPatterns` (regex strings).

## 9. Custom Plugins

Add action hooks in `~/.config/k7s/plugins.yaml` to run shell commands against selected resources. See `CLAUDE.md` for the plugin dispatch model.
