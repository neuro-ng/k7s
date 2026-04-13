# Getting Started with k7s

Welcome to **k7s**, a high-performance, security-first Kubernetes TUI (Terminal UI) with built-in AI capabilities. 

This guide will walk you through the essential steps to get `k7s` up and running, and teach you how to navigate its interface effectively.

## Prerequisites
- **Rust** 1.77+
- **kubectl** available in your `$PATH`
- A valid `kubeconfig` file usually at `~/.kube/config` indicating your connection to a Kubernetes cluster.

## 1. Installation

You can run `k7s` either by compiling it from source or via Docker.

### Building from Source

```bash
git clone https://github.com/your-org/k7s
cd k7s
cargo install --path .
```

### Running with Docker

Mount your kubeconfig file and the k7s config folder to use the pre-built Docker image:

```bash
docker run --rm -it \
  -v "$HOME/.kube:/root/.kube:ro" \
  -v "$HOME/.config/k7s:/root/.config/k7s:ro" \
  ghcr.io/your-org/k7s:latest
```

## 2. Launching k7s

You can start `k7s` directly from your terminal. It will automatically use the active context defined by your `kubeconfig`.

```bash
# Starts the TUI using the active context
k7s

# Connect exactly to a specific context
k7s --context my-cluster

# Open it filtering to watch a particular namespace
k7s --namespace kube-system

# Launch in Read-only mode (prevents accidental deletions/scaling)
k7s --readonly
```

## 3. Navigating the TUI

`k7s` is driven entirely by simple keyboard shortcuts. Type `?` at any time to see the help menu.

### Switching Views
To switch the main resource view, use colon commands (similar to vim):
- `:pod` - View Pods
- `:deploy` - View Deployments
- `:svc` - View Services
- `:node` - View Nodes
- `:ns` - View Namespaces
- `:helm` - View Helm releases

### Action Keys
Use these keys on any selected item to manage the resource:
- `Enter` or `d` - Describe resource
- `l` - Stream logs (useful on pods)
- `s` - Shell into pod, or Scale a deployment
- `y` - View YAML of the active resource
- `Ctrl-d` - Delete resource
- `r` - Restart workloads
- `/` - Open the search/filter dialog
- `q` - Quit the application or go back to the previous screen

## 4. Setting up the AI Assistant

One of `k7s`'s standout features is its built-in AI assistant to help you troubleshoot your cluster quickly via the `:chat` command. Crucially, log secrets and sensitive values are explicitly sanitized and removed before data is passed to the AI platform. 

### To use an OpenAI-compatible API
Export the key and update your configuration file:
```bash
export K7S_LLM_API_KEY="sk-..."
```
Then update `~/.config/k7s/config.yaml`:
```yaml
ai:
  provider: api
  endpoint: https://api.openai.com/v1/chat/completions
  model: gpt-4o
```

### To use Google Antigravity (ADC)
Simply login via gcloud:
```bash
gcloud auth application-default login
```
Set in `config.yaml`:
```yaml
ai:
  provider: antigravity
```

### Using AI Features
- **Open a conversation:** Type `:chat` and ask any question about standard cluster health, such as "Why is that pod failing?".
- **Analyzing a selected pod's logs:** Type `l` to stream logs, then `a` to let the AI summarize exceptions and signals concisely.

## What's Next?
- Check out `~/.config/k7s/config.yaml` to customize refresh rates, skins/themes, and configure sanitizer limits.
- Take a look at `~/.config/k7s/plugins.yaml` if you want to write your own custom action hooks.
