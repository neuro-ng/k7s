//! Custom views configuration — Phase 4.12 / 9.6.
//!
//! Allows users to override the columns shown for any resource type
//! (identified by its GVR string) and to add extra columns that read values
//! via JSON Pointer expressions (RFC 6901, e.g. `/status/podIP`).
//!
//! # File location
//!
//! `~/.config/k7s/views.yaml`
//!
//! # Example
//!
//! ```yaml
//! views:
//!   v1/pods:
//!     columns:
//!       - name: NAME
//!         width: 40
//!       - name: STATUS
//!         width: 12
//!       - name: POD IP
//!         jsonPointer: /status/podIP
//!         width: 18
//!   apps/v1/deployments:
//!     columns:
//!       - name: NAME
//!         width: 40
//!       - name: READY
//!         width: 8
//!       - name: REPLICAS
//!         jsonPointer: /spec/replicas
//!         width: 10
//! ```
//!
//! # k9s Reference: `internal/config/views.go`, `internal/render/cust_col.go`

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;

/// Top-level views configuration file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ViewsConfig {
    /// Key: GVR string (`"v1/pods"`, `"apps/v1/deployments"`, etc.)
    #[serde(default)]
    pub views: HashMap<String, ResourceViewConfig>,
}

/// Per-resource column configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceViewConfig {
    /// Ordered column definitions to show for this resource type.
    #[serde(default)]
    pub columns: Vec<CustomColumnDef>,
}

/// A single column definition in a custom view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomColumnDef {
    /// Column header label.
    pub name: String,
    /// Display width in terminal characters.
    /// `0` means "fill remaining space" (Min constraint).
    #[serde(default = "default_width")]
    pub width: u16,
    /// JSON Pointer expression (RFC 6901) to extract the cell value from the
    /// resource JSON.  E.g. `/status/podIP` or `/metadata/labels/app`.
    ///
    /// When `None` the column is a well-known named column handled by the
    /// base renderer (e.g. `NAME`, `NAMESPACE`, `AGE`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "jsonPointer"
    )]
    pub json_pointer: Option<String>,
}

fn default_width() -> u16 {
    20
}

impl CustomColumnDef {
    /// Extract the cell value from a JSON resource object.
    ///
    /// If a `json_pointer` is set, follows it into the object.
    /// Falls back to the empty string if the pointer finds nothing or the
    /// column is a named (non-pointer) column.
    pub fn extract(&self, obj: &serde_json::Value) -> String {
        if let Some(ptr) = &self.json_pointer {
            obj.pointer(ptr)
                .map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    serde_json::Value::Null => String::new(),
                    other => other.to_string(),
                })
                .unwrap_or_default()
        } else {
            String::new()
        }
    }

    /// Build a [`ratatui::layout::Constraint`] from the configured width.
    pub fn constraint(&self) -> ratatui::layout::Constraint {
        if self.width == 0 {
            ratatui::layout::Constraint::Min(10)
        } else {
            ratatui::layout::Constraint::Length(self.width)
        }
    }
}

impl ViewsConfig {
    /// Load views config from disk.
    ///
    /// Returns an empty config (no overrides) when the file is absent.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            tracing::debug!(path = %path.display(), "views.yaml not found, using built-in column sets");
            return Ok(Self::default());
        }

        let raw = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_owned(),
            source,
        })?;

        serde_yaml::from_str(&raw).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source,
        })
    }

    /// Return the column config for a GVR string, if any.
    ///
    /// The GVR string format is `"<group>/<version>/<resource>"` for grouped
    /// resources (e.g. `"apps/v1/deployments"`) or `"<version>/<resource>"` for
    /// core resources (e.g. `"v1/pods"`).
    pub fn for_gvr(&self, gvr: &str) -> Option<&ResourceViewConfig> {
        self.views.get(gvr)
    }
}

// ─── Custom column renderer wrapper ──────────────────────────────────────────

use crate::client::Gvr;
use crate::render::{ColumnDef, RenderedRow, Renderer};

/// Wraps any `Renderer` and replaces its columns with a custom set from
/// [`ViewsConfig`].
///
/// For named columns (no `json_pointer`) the value falls through to the inner
/// renderer's cell at the same column-name position.  For pointer columns the
/// value is extracted directly from the raw JSON.
///
/// This allows partial overrides: a user can keep the built-in `NAME` and `AGE`
/// columns while adding a custom `/status/podIP` column between them.
pub struct CustomColumnRenderer {
    inner: Box<dyn Renderer>,
    custom_columns: Vec<CustomColumnDef>,
    /// Column definitions pre-built for the `Renderer` trait.
    col_defs: Vec<ColumnDef>,
}

// ColumnDef uses &'static str for names, but custom column names come from the
// config at runtime. We work around this by leaking the strings (there will be
// very few of them — one per configured column per view).
fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_owned().into_boxed_str())
}

impl CustomColumnRenderer {
    pub fn new(inner: Box<dyn Renderer>, columns: Vec<CustomColumnDef>) -> Self {
        let col_defs: Vec<ColumnDef> = columns
            .iter()
            .map(|c| ColumnDef::new(leak_str(&c.name), c.constraint()))
            .collect();
        Self {
            inner,
            custom_columns: columns,
            col_defs,
        }
    }

    /// Build a lookup from column name → inner cell index for fallback.
    fn inner_column_index(&self, name: &str) -> Option<usize> {
        self.inner
            .columns()
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(name))
    }
}

impl Renderer for CustomColumnRenderer {
    fn gvr(&self) -> &Gvr {
        self.inner.gvr()
    }

    fn columns(&self) -> &[ColumnDef] {
        &self.col_defs
    }

    fn render(&self, obj: &serde_json::Value) -> RenderedRow {
        // Get the inner row for fallback values.
        let inner_row = self.inner.render(obj);

        let cells: Vec<String> = self
            .custom_columns
            .iter()
            .map(|col| {
                if col.json_pointer.is_some() {
                    // Pointer column — extract directly from JSON.
                    col.extract(obj)
                } else {
                    // Named column — fall back to inner renderer value.
                    self.inner_column_index(&col.name)
                        .and_then(|idx| inner_row.cells.get(idx))
                        .cloned()
                        .unwrap_or_default()
                }
            })
            .collect();

        RenderedRow {
            cells,
            age_secs: inner_row.age_secs,
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_views_config_is_empty() {
        let cfg = ViewsConfig::default();
        assert!(cfg.views.is_empty());
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let cfg = ViewsConfig::load(Path::new("/nonexistent/views.yaml")).unwrap();
        assert!(cfg.views.is_empty());
    }

    #[test]
    fn load_valid_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("views.yaml");
        std::fs::write(
            &path,
            r#"
views:
  v1/pods:
    columns:
      - name: NAME
        width: 40
      - name: POD IP
        jsonPointer: /status/podIP
        width: 18
"#,
        )
        .unwrap();
        let cfg = ViewsConfig::load(&path).unwrap();
        let view = cfg.for_gvr("v1/pods").expect("v1/pods should be present");
        assert_eq!(view.columns.len(), 2);
        assert_eq!(view.columns[0].name, "NAME");
        assert!(view.columns[0].json_pointer.is_none());
        assert_eq!(view.columns[1].name, "POD IP");
        assert_eq!(
            view.columns[1].json_pointer.as_deref(),
            Some("/status/podIP")
        );
    }

    #[test]
    fn custom_column_extract_string() {
        let col = CustomColumnDef {
            name: "IP".to_owned(),
            width: 16,
            json_pointer: Some("/status/podIP".to_owned()),
        };
        let obj = serde_json::json!({ "status": { "podIP": "10.0.0.1" } });
        assert_eq!(col.extract(&obj), "10.0.0.1");
    }

    #[test]
    fn custom_column_extract_number() {
        let col = CustomColumnDef {
            name: "REPLICAS".to_owned(),
            width: 10,
            json_pointer: Some("/spec/replicas".to_owned()),
        };
        let obj = serde_json::json!({ "spec": { "replicas": 3 } });
        assert_eq!(col.extract(&obj), "3");
    }

    #[test]
    fn custom_column_extract_missing_path_returns_empty() {
        let col = CustomColumnDef {
            name: "X".to_owned(),
            width: 10,
            json_pointer: Some("/does/not/exist".to_owned()),
        };
        let obj = serde_json::json!({});
        assert_eq!(col.extract(&obj), "");
    }

    #[test]
    fn custom_column_no_pointer_returns_empty_on_extract() {
        let col = CustomColumnDef {
            name: "NAME".to_owned(),
            width: 40,
            json_pointer: None,
        };
        let obj = serde_json::json!({ "metadata": { "name": "my-pod" } });
        // Named columns fall through to inner renderer in CustomColumnRenderer.
        // direct extract() call returns empty.
        assert_eq!(col.extract(&obj), "");
    }

    #[test]
    fn custom_column_renderer_pointer_column() {
        use crate::render::PodRenderer;

        let inner: Box<dyn Renderer> = Box::new(PodRenderer::new());
        let cols = vec![
            CustomColumnDef {
                name: "NAME".to_owned(),
                width: 40,
                json_pointer: None,
            },
            CustomColumnDef {
                name: "POD IP".to_owned(),
                width: 16,
                json_pointer: Some("/status/podIP".to_owned()),
            },
        ];
        let renderer = CustomColumnRenderer::new(inner, cols);

        let obj = serde_json::json!({
            "metadata": {
                "name": "test-pod",
                "namespace": "default",
                "creationTimestamp": "2024-01-01T00:00:00Z"
            },
            "status": {
                "podIP": "10.0.1.5",
                "phase": "Running"
            }
        });

        let row = renderer.render(&obj);
        assert_eq!(row.cells.len(), 2);
        assert_eq!(row.cells[0], "test-pod"); // NAME fallback
        assert_eq!(row.cells[1], "10.0.1.5"); // pointer extraction
    }

    #[test]
    fn constraint_zero_width_becomes_min() {
        let col = CustomColumnDef {
            name: "X".to_owned(),
            width: 0,
            json_pointer: None,
        };
        assert!(matches!(
            col.constraint(),
            ratatui::layout::Constraint::Min(_)
        ));
    }

    #[test]
    fn constraint_nonzero_width_becomes_length() {
        let col = CustomColumnDef {
            name: "X".to_owned(),
            width: 20,
            json_pointer: None,
        };
        assert!(matches!(
            col.constraint(),
            ratatui::layout::Constraint::Length(20)
        ));
    }
}
