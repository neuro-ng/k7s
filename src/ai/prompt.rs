//! Prompt template engine — Phase 12.5.
//!
//! Provides resource-specific prompt templates for each AI capability.
//! Every template:
//! 1. Declares the task clearly for the LLM.
//! 2. Incorporates sanitized cluster context.
//! 3. Requests a structured, concise response.
//!
//! Templates return a `String` that becomes the **user message** sent to the
//! `ChatSession`. The system prompt (k7s identity) is already added by the
//! session layer.
//!
//! # k7s-unique — no k9s equivalent.

use crate::sanitizer::SafeMetadata;

// ─── Template kinds ───────────────────────────────────────────────────────────

/// All available analysis templates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptKind {
    /// Diagnose why a pod / workload is unhealthy.
    ErrorAnalysis,
    /// Identify over-provisioned resources and savings opportunities.
    EfficiencyRecommendations,
    /// Overall cluster health assessment.
    ClusterHealth,
    /// RBAC permission audit suggestions.
    RbacReview,
    /// Log root-cause analysis (log summary already compressed).
    LogTroubleshooting,
    /// Free-form chat — no structured template.
    General,
}

impl PromptKind {
    /// Human-readable label shown in the chat UI.
    pub fn label(&self) -> &'static str {
        match self {
            PromptKind::ErrorAnalysis => "Error Analysis",
            PromptKind::EfficiencyRecommendations => "Efficiency Recommendations",
            PromptKind::ClusterHealth => "Cluster Health",
            PromptKind::RbacReview => "RBAC Review",
            PromptKind::LogTroubleshooting => "Log Troubleshooting",
            PromptKind::General => "General",
        }
    }
}

// ─── Template builder ─────────────────────────────────────────────────────────

/// Build a user-message prompt for `kind` using `context` as supporting data.
///
/// `extra` is an optional free-form addition supplied by the user (e.g. a
/// specific question or log excerpt they pasted).
pub fn build(kind: &PromptKind, context: &[SafeMetadata], extra: Option<&str>) -> String {
    let ctx_block = format_context(context);

    let task = match kind {
        PromptKind::ErrorAnalysis => error_analysis_task(),
        PromptKind::EfficiencyRecommendations => efficiency_task(),
        PromptKind::ClusterHealth => cluster_health_task(),
        PromptKind::RbacReview => rbac_review_task(),
        PromptKind::LogTroubleshooting => log_troubleshooting_task(),
        PromptKind::General => {
            return extra
                .unwrap_or("Tell me about the cluster state shown in context.")
                .to_owned();
        }
    };

    let mut prompt = format!("{task}\n\n{ctx_block}");
    if let Some(e) = extra {
        if !e.trim().is_empty() {
            prompt.push_str("\n\n## Additional context\n\n");
            prompt.push_str(e);
        }
    }
    prompt.push_str("\n\nPlease respond concisely. Use markdown. Lead with your diagnosis, then list actionable next steps.");
    prompt
}

// ─── Task instructions ────────────────────────────────────────────────────────

fn error_analysis_task() -> &'static str {
    "## Task: Error Analysis\n\n\
     Analyse the Kubernetes resources below and diagnose why the workload is \
     unhealthy or failing. Focus on:\n\
     - Pod phase, conditions, and container statuses\n\
     - Events (reason, message, count)\n\
     - Restart counts and last exit codes\n\
     - Image pull failures or OOMKills\n\
     Identify the root cause and suggest remediation steps."
}

fn efficiency_task() -> &'static str {
    "## Task: Efficiency Recommendations\n\n\
     Review the resource requests and limits in the workloads below and identify \
     efficiency improvements. Consider:\n\
     - Over-provisioned CPU/memory requests vs actual usage\n\
     - Missing resource limits (risk of noisy-neighbour)\n\
     - Idle deployments (0 ready pods, long age)\n\
     - Opportunities for Horizontal Pod Autoscaling\n\
     Provide specific, quantified recommendations where possible."
}

fn cluster_health_task() -> &'static str {
    "## Task: Cluster Health Assessment\n\n\
     Assess the overall health of the cluster based on the information below. \
     Evaluate:\n\
     - Node conditions (Ready, MemoryPressure, DiskPressure, PIDPressure)\n\
     - Critical workload availability (control-plane, monitoring)\n\
     - Recent warning/error events and their frequency\n\
     - Namespace resource consumption patterns\n\
     Summarise the health with a RAG (Red/Amber/Green) status and list top risks."
}

fn rbac_review_task() -> &'static str {
    "## Task: RBAC Review\n\n\
     Review the RBAC configuration below and flag potential security concerns. \
     Look for:\n\
     - Overly broad ClusterRoles (wildcards on resources or verbs)\n\
     - Default ServiceAccounts with non-trivial bindings\n\
     - Roles granting secrets access to non-privileged subjects\n\
     - Bindings across namespaces that may violate least-privilege\n\
     Note: secret values are never included — only role/binding structure is shown."
}

fn log_troubleshooting_task() -> &'static str {
    "## Task: Log Troubleshooting\n\n\
     Analyse the compressed log summary below and identify the root cause of any \
     errors or anomalies. Consider:\n\
     - Error frequency and clustering (similar messages grouped)\n\
     - Error level distribution (ERR%, WARN%, INFO%)\n\
     - Temporal patterns (sudden spikes, recurring intervals)\n\
     - Stack trace excerpts that indicate the failing component\n\
     Suggest the most likely cause and what to investigate next."
}

// ─── Context formatter ────────────────────────────────────────────────────────

/// Format a list of `SafeMetadata` entries into a fenced markdown block.
fn format_context(context: &[SafeMetadata]) -> String {
    if context.is_empty() {
        return "*(no cluster context provided)*".to_owned();
    }

    let mut out = "## Cluster context (sanitised)\n\n".to_owned();
    for meta in context {
        let ns_name = match &meta.namespace {
            Some(ns) => format!("{ns}/{}", meta.name),
            None => meta.name.clone(),
        };
        let json = serde_json::to_string_pretty(&meta.fields).unwrap_or_else(|_| "{}".to_owned());
        out.push_str(&format!(
            "### {} `{ns_name}`\n```json\n{json}\n```\n\n",
            meta.gvr,
        ));
    }
    out
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta(name: &str) -> SafeMetadata {
        SafeMetadata {
            gvr: "v1/pods".to_owned(),
            namespace: Some("default".to_owned()),
            name: name.to_owned(),
            fields: json!({"status": {"phase": "CrashLoopBackOff"}}),
        }
    }

    #[test]
    fn error_analysis_contains_task_header() {
        let prompt = build(&PromptKind::ErrorAnalysis, &[meta("nginx")], None);
        assert!(prompt.contains("Error Analysis"));
        assert!(prompt.contains("nginx"));
        assert!(prompt.contains("actionable next steps"));
    }

    #[test]
    fn general_kind_returns_extra_verbatim() {
        let prompt = build(&PromptKind::General, &[], Some("What is the pod count?"));
        assert_eq!(prompt, "What is the pod count?");
    }

    #[test]
    fn general_kind_fallback_when_no_extra() {
        let prompt = build(&PromptKind::General, &[], None);
        assert!(prompt.contains("cluster state"));
    }

    #[test]
    fn efficiency_prompt_contains_requests_limits_guidance() {
        let prompt = build(&PromptKind::EfficiencyRecommendations, &[meta("api")], None);
        assert!(prompt.contains("resource requests"));
        assert!(prompt.contains("limits"));
    }

    #[test]
    fn cluster_health_mentions_rag() {
        let prompt = build(&PromptKind::ClusterHealth, &[meta("node-1")], None);
        assert!(prompt.contains("RAG"));
    }

    #[test]
    fn rbac_review_mentions_no_secrets() {
        let prompt = build(&PromptKind::RbacReview, &[meta("admin-binding")], None);
        assert!(prompt.contains("secret values are never included"));
    }

    #[test]
    fn log_troubleshooting_mentions_compression() {
        let prompt = build(&PromptKind::LogTroubleshooting, &[meta("worker")], None);
        assert!(prompt.contains("compressed log summary"));
    }

    #[test]
    fn extra_context_appended() {
        let prompt = build(
            &PromptKind::ErrorAnalysis,
            &[meta("crash-pod")],
            Some("OOMKilled 3 times in the last hour"),
        );
        assert!(prompt.contains("OOMKilled 3 times"));
    }

    #[test]
    fn empty_context_shows_placeholder() {
        let prompt = build(&PromptKind::ErrorAnalysis, &[], None);
        assert!(prompt.contains("no cluster context provided"));
    }

    #[test]
    fn format_context_includes_gvr_and_json() {
        let ctx = vec![meta("my-pod")];
        let out = format_context(&ctx);
        assert!(out.contains("v1/pods"));
        assert!(out.contains("my-pod"));
        assert!(out.contains("CrashLoopBackOff"));
    }

    #[test]
    fn prompt_kind_labels() {
        assert_eq!(PromptKind::ErrorAnalysis.label(), "Error Analysis");
        assert_eq!(PromptKind::ClusterHealth.label(), "Cluster Health");
    }
}
