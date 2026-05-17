//! KubeVela / OAM renderers — Phase 36.
//!
//! Standalone renderers for VelaApplication, VelaComponent, VelaWorkflowStep,
//! VelaRevision, and VelaDefinition. These do not implement the `Renderer`
//! trait because they are not native Kubernetes resources.

use ratatui::style::{Color, Style};

use crate::render::RenderedRow;
use crate::vela::{VelaApplication, VelaComponent, VelaDefinition, VelaRevision, VelaWorkflowStep};

// ─── Application table ────────────────────────────────────────────────────────

pub fn app_headers() -> Vec<&'static str> {
    vec![
        "NAME",
        "NAMESPACE",
        "STATUS",
        "WORKFLOW",
        "COMPONENTS",
        "AGE",
    ]
}

pub fn render_app(app: &VelaApplication) -> RenderedRow {
    RenderedRow {
        cells: vec![
            app.name.clone(),
            app.namespace.clone(),
            app.status.clone(),
            app.workflow_status.clone(),
            app.component_count.to_string(),
            app.age_label(),
        ],
        age_secs: app.age_secs,
    }
}

// ─── Component table ──────────────────────────────────────────────────────────

pub fn component_headers() -> Vec<&'static str> {
    vec!["NAME", "TYPE", "HEALTHY", "TRAITS", "MESSAGE"]
}

pub fn render_component(comp: &VelaComponent) -> RenderedRow {
    let healthy = if comp.healthy { "true" } else { "false" };
    let traits = comp
        .traits
        .iter()
        .map(|t| t.trait_type.as_str())
        .collect::<Vec<_>>()
        .join(",");
    RenderedRow {
        cells: vec![
            comp.name.clone(),
            comp.workload_type.clone(),
            healthy.to_string(),
            traits,
            comp.message.clone(),
        ],
        age_secs: 0,
    }
}

// ─── Workflow step table ──────────────────────────────────────────────────────

pub fn workflow_headers() -> Vec<&'static str> {
    vec!["NAME", "TYPE", "PHASE", "MESSAGE"]
}

pub fn render_workflow_step(step: &VelaWorkflowStep) -> RenderedRow {
    RenderedRow {
        cells: vec![
            step.name.clone(),
            step.step_type.clone(),
            step.phase.clone(),
            step.message.clone(),
        ],
        age_secs: 0,
    }
}

// ─── Revision table ───────────────────────────────────────────────────────────

pub fn revision_headers() -> Vec<&'static str> {
    vec!["REVISION", "NAME", "DEPLOY TIME", "STATUS"]
}

pub fn render_revision(rev: &VelaRevision) -> RenderedRow {
    RenderedRow {
        cells: vec![
            rev.revision.to_string(),
            rev.name.clone(),
            rev.deploy_time.clone(),
            rev.status.clone(),
        ],
        age_secs: 0,
    }
}

// ─── Definition table ─────────────────────────────────────────────────────────

pub fn def_headers() -> Vec<&'static str> {
    vec!["NAME", "TYPE", "DESCRIPTION"]
}

pub fn render_definition(def: &VelaDefinition) -> RenderedRow {
    RenderedRow {
        cells: vec![
            def.name.clone(),
            def.def_type.clone(),
            def.description.clone(),
        ],
        age_secs: 0,
    }
}

// ─── Colour helpers ───────────────────────────────────────────────────────────

/// Style for an Application overall status string.
pub fn status_color(status: &str) -> Style {
    match status.to_lowercase().as_str() {
        "running" => Style::default().fg(Color::Green),
        "workflowfailed" | "workflowsuspending" | "deleting" => Style::default().fg(Color::Red),
        "workflowrunning" | "rendering" | "starting" => Style::default().fg(Color::Yellow),
        _ => Style::default().fg(Color::DarkGray),
    }
}

/// Style for a workflow step phase string.
pub fn phase_color(phase: &str) -> Style {
    match phase.to_lowercase().as_str() {
        "succeeded" => Style::default().fg(Color::Green),
        "failed" => Style::default().fg(Color::Red),
        "running" => Style::default().fg(Color::Yellow),
        "skipped" | "stopped" => Style::default().fg(Color::DarkGray),
        _ => Style::default().fg(Color::White),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vela::{VelaApplication, VelaComponent, VelaRevision, VelaWorkflowStep};
    use ratatui::style::Color;
    use serde_json::json;

    fn make_app() -> VelaApplication {
        VelaApplication {
            name: "my-app".into(),
            namespace: "default".into(),
            status: "running".into(),
            component_count: 2,
            workflow_status: "succeeded".into(),
            age_secs: 3600,
            raw: json!({}),
        }
    }

    #[test]
    fn app_headers_count() {
        assert_eq!(app_headers().len(), 6);
    }

    #[test]
    fn render_app_cells() {
        let row = render_app(&make_app());
        assert_eq!(row.cells[0], "my-app");
        assert_eq!(row.cells[1], "default");
        assert_eq!(row.cells[2], "running");
        assert_eq!(row.cells[4], "2");
        assert_eq!(row.cells[5], "60m");
    }

    #[test]
    fn component_headers_count() {
        assert_eq!(component_headers().len(), 5);
    }

    #[test]
    fn render_component_healthy() {
        let comp = VelaComponent {
            name: "web".into(),
            workload_type: "webservice".into(),
            healthy: true,
            message: "".into(),
            traits: vec![],
        };
        let row = render_component(&comp);
        assert_eq!(row.cells[2], "true");
    }

    #[test]
    fn render_component_traits_joined() {
        let comp = VelaComponent {
            name: "web".into(),
            workload_type: "webservice".into(),
            healthy: true,
            message: "".into(),
            traits: vec![
                crate::vela::VelaTrait {
                    trait_type: "scaler".into(),
                    healthy: true,
                    message: "".into(),
                },
                crate::vela::VelaTrait {
                    trait_type: "rollout".into(),
                    healthy: true,
                    message: "".into(),
                },
            ],
        };
        let row = render_component(&comp);
        assert_eq!(row.cells[3], "scaler,rollout");
    }

    #[test]
    fn workflow_headers_count() {
        assert_eq!(workflow_headers().len(), 4);
    }

    #[test]
    fn render_workflow_step_cells() {
        let step = VelaWorkflowStep {
            name: "deploy".into(),
            step_type: "deploy".into(),
            phase: "succeeded".into(),
            message: "".into(),
        };
        let row = render_workflow_step(&step);
        assert_eq!(row.cells[0], "deploy");
        assert_eq!(row.cells[2], "succeeded");
    }

    #[test]
    fn revision_headers_count() {
        assert_eq!(revision_headers().len(), 4);
    }

    #[test]
    fn render_revision_cells() {
        let rev = VelaRevision {
            name: "my-app-v3".into(),
            revision: 3,
            deploy_time: "2026-05-01 10:00".into(),
            status: "succeeded".into(),
        };
        let row = render_revision(&rev);
        assert_eq!(row.cells[0], "3");
        assert_eq!(row.cells[3], "succeeded");
    }

    #[test]
    fn def_headers_count() {
        assert_eq!(def_headers().len(), 3);
    }

    #[test]
    fn status_color_running_is_green() {
        let style = status_color("running");
        assert_eq!(style.fg, Some(Color::Green));
    }

    #[test]
    fn status_color_failed_is_red() {
        let style = status_color("workflowFailed");
        assert_eq!(style.fg, Some(Color::Red));
    }

    #[test]
    fn phase_color_succeeded_is_green() {
        let style = phase_color("succeeded");
        assert_eq!(style.fg, Some(Color::Green));
    }

    #[test]
    fn phase_color_running_is_yellow() {
        let style = phase_color("running");
        assert_eq!(style.fg, Some(Color::Yellow));
    }
}
