//! Animated logo cross-dissolve transition — Phase 27.
//!
//! Implements a four-stage dissolve effect between consecutive logo frames
//! in the TUI header.  Each tick (100 ms) advances the active stage by
//! `speed` cells.  The `speed` value maps to `config.k7s.ui.logo_transition_speed`
//! (default 3, range 1–10).
//!
//! # Stage sequence
//!
//! ```text
//! [Hold] ──▶ [DotFill] ──▶ [InterlaceIn] ──▶ [PlusDissolve] ──▶ [Hold] ──▶ …
//!             spaces→'.'   dots→frame_B        '+'→space
//!                          frame_A chars→'+'
//! ```
//!
//! See `TransitionBuffer` for the cell encoding and `step()` for per-tick logic.

use crate::ui::splash::LOGO_FRAMES;

/// Current phase of the cross-dissolve animation.
#[derive(Debug, Clone)]
pub enum TransitionState {
    /// Displaying the current frame statically for `remaining` ticks.
    Hold { remaining: usize },
    /// Stage 1: replacing space cells with `.`, sweeping left→right, bottom→top.
    DotFill { progress: usize },
    /// Stage 2: replacing `.` with frame_B chars; frame_A foreground cells → `+`.
    InterlaceIn { progress: usize },
    /// Stage 3: replacing `+` cells with ` `, sweeping top→bottom, left→right.
    PlusDissolve { progress: usize },
}

/// Marker characters used in the composite buffer.
const DOT: char = '.';
const PLUS: char = '+';

/// A composite 2-D grid that holds the current transitioning display state.
///
/// Cells encode both the character to render and which "layer" it belongs to,
/// allowing the renderer to colour them differently:
///
/// | Cell char | Colour                      |
/// |-----------|---------------------------  |
/// | `' '`     | transparent (background)    |
/// | `'.'`     | DarkGray (dissolving space) |
/// | `'+'`     | Yellow (dissolving old char)|
/// | anything else | `logo_frame_color(next_frame)` |
#[derive(Debug, Clone)]
pub struct TransitionBuffer {
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<Vec<char>>,
}

impl TransitionBuffer {
    /// Allocate a buffer sized for the largest frame in `LOGO_FRAMES`.
    pub fn new() -> Self {
        let rows = LOGO_FRAMES.iter().map(|f| f.len()).max().unwrap_or(6);
        let cols = LOGO_FRAMES
            .iter()
            .flat_map(|f| f.iter())
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(36)
            + 2; // extra padding so we never clip
        Self {
            rows,
            cols,
            cells: vec![vec![' '; cols]; rows],
        }
    }

    /// Load `frame_idx` from `LOGO_FRAMES` into the buffer (pad short lines).
    pub fn load_frame(&mut self, frame_idx: usize) {
        let frame = LOGO_FRAMES[frame_idx % LOGO_FRAMES.len()];
        for r in 0..self.rows {
            let line: Vec<char> = frame
                .get(r)
                .map(|l| l.chars().collect())
                .unwrap_or_default();
            for c in 0..self.cols {
                self.cells[r][c] = line.get(c).copied().unwrap_or(' ');
            }
        }
    }

    /// Render the buffer as a flat string per row (for test assertions).
    #[cfg(test)]
    pub fn row_str(&self, row: usize) -> String {
        self.cells[row].iter().collect()
    }
}

impl Default for TransitionBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Advance the transition by one tick, modifying `buf` in place.
///
/// Returns the next `TransitionState` (may be the same state with updated
/// progress, or the following stage when a stage completes).
///
/// `speed` is the number of cells to advance per tick (clamped 1..=10).
pub fn step(
    state: TransitionState,
    buf: &mut TransitionBuffer,
    frame_a: usize,
    frame_b: usize,
    speed: usize,
    hold_ticks: usize,
) -> TransitionState {
    let speed = speed.clamp(1, 10);
    match state {
        TransitionState::Hold { remaining } => {
            if remaining == 0 {
                // Start Stage 1: load frame_A cleanly, then begin dot fill.
                buf.load_frame(frame_a);
                TransitionState::DotFill { progress: 0 }
            } else {
                TransitionState::Hold {
                    remaining: remaining - 1,
                }
            }
        }

        TransitionState::DotFill { progress } => {
            let new_progress = dot_fill_step(buf, progress, speed);
            let total_spaces = count_cells(buf, ' ') + new_progress;
            if new_progress >= total_spaces || count_cells(buf, ' ') == 0 {
                // All spaces filled — move to Stage 2.
                TransitionState::InterlaceIn { progress: 0 }
            } else {
                TransitionState::DotFill {
                    progress: new_progress,
                }
            }
        }

        TransitionState::InterlaceIn { progress } => {
            let new_progress = interlace_step(buf, frame_a, frame_b, progress, speed);
            if count_cells(buf, DOT) == 0 {
                // All dots replaced — move to Stage 3.
                TransitionState::PlusDissolve { progress: 0 }
            } else {
                TransitionState::InterlaceIn {
                    progress: new_progress,
                }
            }
        }

        TransitionState::PlusDissolve { progress } => {
            let new_progress = plus_dissolve_step(buf, progress, speed);
            if count_cells(buf, PLUS) == 0 {
                // All pluses dissolved — hold on frame_B.
                buf.load_frame(frame_b);
                TransitionState::Hold {
                    remaining: hold_ticks,
                }
            } else {
                TransitionState::PlusDissolve {
                    progress: new_progress,
                }
            }
        }
    }
}

/// Stage 1: replace `speed` space cells with `.`, sweeping left→right, bottom→top.
/// Returns the new progress (total cells filled so far).
fn dot_fill_step(buf: &mut TransitionBuffer, progress: usize, speed: usize) -> usize {
    let mut filled = 0;
    // Bottom-up, left-right sweep.
    'outer: for r in (0..buf.rows).rev() {
        for c in 0..buf.cols {
            if buf.cells[r][c] == ' ' {
                // Only fill cells that are background spaces, not logo chars.
                // Compute global position and check if we're past `progress`.
                let pos = (buf.rows - 1 - r) * buf.cols + c;
                if pos >= progress {
                    buf.cells[r][c] = DOT;
                    filled += 1;
                    if filled >= speed {
                        break 'outer;
                    }
                }
            }
        }
    }
    progress + filled
}

/// Stage 2: replace `speed` dot cells with frame_B chars; frame_A foreground → `+`.
/// Sweeps top→bottom, left→right.
///
/// No progress-offset guard is needed here: replaced dots are no longer `'.'`,
/// so they are naturally skipped on subsequent ticks.
fn interlace_step(
    buf: &mut TransitionBuffer,
    _frame_a: usize,
    frame_b: usize,
    progress: usize,
    speed: usize,
) -> usize {
    let b_frame = LOGO_FRAMES[frame_b % LOGO_FRAMES.len()];
    let mut replaced = 0;

    'outer: for r in 0..buf.rows {
        let b_line: Vec<char> = b_frame
            .get(r)
            .map(|l| l.chars().collect())
            .unwrap_or_default();

        for c in 0..buf.cols {
            if buf.cells[r][c] == DOT {
                // Replace dot with corresponding frame_B char (or space).
                buf.cells[r][c] = b_line.get(c).copied().unwrap_or(' ');
                replaced += 1;
                if replaced >= speed {
                    break 'outer;
                }
            } else if buf.cells[r][c] != ' ' && buf.cells[r][c] != PLUS {
                // Frame_A foreground character — convert to `+` as it's "replaced".
                let b_char = b_line.get(c).copied().unwrap_or(' ');
                if b_char != buf.cells[r][c] {
                    buf.cells[r][c] = PLUS;
                }
            }
        }
    }
    progress + replaced
}

/// Stage 3: replace `speed` `+` cells with ` `, sweeping top→bottom, left→right.
///
/// No progress-offset guard: cleared `+` cells become spaces and are skipped
/// naturally on the next tick.
fn plus_dissolve_step(buf: &mut TransitionBuffer, progress: usize, speed: usize) -> usize {
    let mut cleared = 0;

    'outer: for r in 0..buf.rows {
        for c in 0..buf.cols {
            if buf.cells[r][c] == PLUS {
                buf.cells[r][c] = ' ';
                cleared += 1;
                if cleared >= speed {
                    break 'outer;
                }
            }
        }
    }
    progress + cleared
}

/// Count cells matching `target` in the buffer.
fn count_cells(buf: &TransitionBuffer, target: char) -> usize {
    buf.cells
        .iter()
        .flat_map(|r| r.iter())
        .filter(|&&c| c == target)
        .count()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_sized_for_largest_frame() {
        let buf = TransitionBuffer::new();
        assert!(buf.rows >= 6);
        assert!(buf.cols >= 24);
    }

    #[test]
    fn load_frame_fills_correctly() {
        let mut buf = TransitionBuffer::new();
        buf.load_frame(0); // BOLD frame
                           // First line of BOLD starts with ' '
        assert_eq!(buf.cells[0][0], ' ');
        // A logo char should appear somewhere.
        let has_block = buf.cells[0].contains(&'█');
        assert!(has_block, "BOLD frame should contain block characters");
    }

    #[test]
    fn dot_fill_replaces_spaces() {
        let mut buf = TransitionBuffer::new();
        buf.load_frame(0);
        let spaces_before = count_cells(&buf, ' ');
        let _ = dot_fill_step(&mut buf, 0, 5);
        let spaces_after = count_cells(&buf, ' ');
        assert!(
            spaces_after < spaces_before,
            "dot_fill_step must replace spaces with '.'"
        );
        let dots = count_cells(&buf, DOT);
        assert!(dots > 0);
    }

    #[test]
    fn dot_fill_never_touches_logo_chars() {
        let mut buf = TransitionBuffer::new();
        buf.load_frame(0);
        let logo_chars_before: Vec<Vec<char>> = buf.cells.clone();
        let _ = dot_fill_step(&mut buf, 0, 999);
        // Logo chars (non-space) must remain unchanged.
        for (r, row_before) in logo_chars_before.iter().enumerate() {
            for (c, &ch_before) in row_before.iter().enumerate() {
                if ch_before != ' ' {
                    assert_ne!(
                        buf.cells[r][c], DOT,
                        "logo char at ({r},{c}) was overwritten with dot"
                    );
                }
            }
        }
    }

    #[test]
    fn plus_dissolve_clears_plus_cells() {
        let mut buf = TransitionBuffer::new();
        // Manually plant some + chars.
        buf.cells[0][0] = PLUS;
        buf.cells[1][0] = PLUS;
        let _ = plus_dissolve_step(&mut buf, 0, 10);
        assert_eq!(count_cells(&buf, PLUS), 0);
        assert_eq!(buf.cells[0][0], ' ');
    }

    #[test]
    fn hold_state_decrements() {
        let mut buf = TransitionBuffer::new();
        buf.load_frame(0);
        let state = TransitionState::Hold { remaining: 3 };
        let next = step(state, &mut buf, 0, 1, 3, 8);
        assert!(matches!(next, TransitionState::Hold { remaining: 2 }));
    }

    #[test]
    fn hold_zero_transitions_to_dot_fill() {
        let mut buf = TransitionBuffer::new();
        buf.load_frame(0);
        let state = TransitionState::Hold { remaining: 0 };
        let next = step(state, &mut buf, 0, 1, 3, 8);
        assert!(matches!(next, TransitionState::DotFill { .. }));
    }

    #[test]
    fn full_dot_fill_transitions_to_interlace() {
        let mut buf = TransitionBuffer::new();
        buf.load_frame(0);
        // Fill all spaces with a huge speed value.
        let state = TransitionState::DotFill { progress: 0 };
        // Run many steps until interlace.
        let mut s = state;
        for _ in 0..1000 {
            s = step(s, &mut buf, 0, 1, 1000, 8);
            if matches!(s, TransitionState::InterlaceIn { .. }) {
                break;
            }
        }
        assert!(matches!(s, TransitionState::InterlaceIn { .. }));
    }

    #[test]
    fn count_cells_accuracy() {
        let mut buf = TransitionBuffer::new();
        buf.load_frame(0);
        let spaces = count_cells(&buf, ' ');
        let non_spaces: usize = buf
            .cells
            .iter()
            .flat_map(|r| r.iter())
            .filter(|&&c| c != ' ')
            .count();
        assert_eq!(spaces + non_spaces, buf.rows * buf.cols);
    }

    /// Regression: interlace and plus-dissolve must complete even with a small speed,
    /// not stall because a growing progress counter exceeds remaining cells.
    #[test]
    fn full_transition_cycle_completes() {
        let mut buf = TransitionBuffer::new();
        buf.load_frame(0);
        let mut s = TransitionState::Hold { remaining: 0 };
        let mut saw_hold_after_dissolve = false;

        for _ in 0..5000 {
            s = step(s, &mut buf, 0, 1, 3, 4);
            if let TransitionState::Hold { remaining: 4 } = s {
                saw_hold_after_dissolve = true;
                break;
            }
        }
        assert!(
            saw_hold_after_dissolve,
            "full transition cycle must complete"
        );
    }

    /// Regression: after one full cycle the animation continues to the next frame.
    #[test]
    fn second_cycle_begins_after_first() {
        let mut buf = TransitionBuffer::new();
        buf.load_frame(0);
        let mut s = TransitionState::Hold { remaining: 0 };
        let mut cycles = 0;

        for _ in 0..20_000 {
            let prev = matches!(s, TransitionState::Hold { remaining: 4 });
            s = step(s, &mut buf, cycles % 6, (cycles + 1) % 6, 3, 4);
            if !prev {
                if let TransitionState::Hold { remaining: 4 } = s {
                    cycles += 1;
                    if cycles >= 3 {
                        break;
                    }
                    // restart from Hold{0} to chain cycles
                    s = TransitionState::Hold { remaining: 0 };
                }
            }
        }
        assert!(
            cycles >= 3,
            "animation must cycle at least 3 times, got {cycles}"
        );
    }
}
