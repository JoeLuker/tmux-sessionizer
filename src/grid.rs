use crate::{
    tmux::Tmux,
    Result,
};

/// A pane to create in the grid — command to run + label for the border.
pub struct GridPane {
    pub command: String,
    pub label: String,
}

/// Build a 3×2 pane grid across one or more tmux windows.
///
/// If `session_name` is Some, creates a new session with that name.
/// If `session_name` is None, adds windows to the current session.
/// Each window gets up to `panes_per_window` panes (default 6) in a 3-column × 2-row layout.
///
/// Split strategy: horizontal splits first (columns), then vertical splits (rows).
/// This avoids the "no space for new pane" error that occurs with sequential
/// vertical splits on terminals shorter than ~50 rows.
pub fn build_pane_grid(
    tmux: &Tmux,
    session_name: Option<&str>,
    panes: Vec<GridPane>,
    panes_per_window: usize,
) -> Result<()> {
    if panes.is_empty() {
        return Ok(());
    }

    let titled_cmd = |pane: &GridPane| -> String {
        format!(
            "printf '\\033]2;{}\\033\\\\'; {}",
            pane.label.replace('\'', ""),
            pane.command
        )
    };

    // Determine the target session
    let target_session: String = if let Some(name) = session_name {
        tmux.kill_session(name);
        let output = tmux.new_session(Some(name), None, Some(&titled_cmd(&panes[0])));
        if !output.status.success() {
            eprintln!("tms: failed to create session '{}': {}",
                name, String::from_utf8_lossy(&output.stderr));
        }
        name.to_string()
    } else {
        let current = tmux.display_message("'#S'").trim().replace('\'', "");
        let output = tmux.new_window(None, None, Some(&current));
        if !output.status.success() {
            eprintln!("tms: failed to create window: {}",
                String::from_utf8_lossy(&output.stderr));
        }
        tmux.send_keys(&titled_cmd(&panes[0]), None);
        current
    };

    // Process panes in chunks of panes_per_window
    let chunks: Vec<&[GridPane]> = panes.chunks(panes_per_window).collect();

    for (chunk_idx, chunk) in chunks.iter().enumerate() {
        if chunk_idx == 0 {
            // First chunk: pane 0 already exists (created with session/window above)
            fill_window(tmux, &target_session, &chunk[1..], &titled_cmd);
        } else {
            // New window for subsequent chunks
            let output = tmux.new_window(None, None, Some(&target_session));
            if !output.status.success() {
                eprintln!("tms: failed to create window: {}",
                    String::from_utf8_lossy(&output.stderr));
            }
            tmux.send_keys(&titled_cmd(&chunk[0]), None);
            fill_window(tmux, &target_session, &chunk[1..], &titled_cmd);
        }
    }

    // Final tiled layout on all windows
    let windows_output = tmux.list_windows("#{window_index}", Some(&target_session));
    for win_idx in windows_output.lines() {
        let win_idx = win_idx.trim();
        if !win_idx.is_empty() {
            let target = format!("{}:{}", target_session, win_idx);
            tmux.select_layout(&target, "tiled");
        }
    }

    Ok(())
}

/// Fill the current window with additional panes using a column-first strategy.
///
/// Strategy for up to 5 additional panes (6 total including the existing one):
/// 1. Split horizontally to create columns (up to 2 more = 3 columns)
/// 2. Even out columns with even-horizontal layout
/// 3. Split each column vertically to create rows
fn fill_window(
    tmux: &Tmux,
    target_session: &str,
    remaining_panes: &[GridPane],
    titled_cmd: &dyn Fn(&GridPane) -> String,
) {
    if remaining_panes.is_empty() {
        return;
    }

    let total = remaining_panes.len() + 1; // +1 for the pane that already exists
    let cols = if total <= 2 { total } else { 3.min(total) };
    let _rows = (total + cols - 1) / cols;

    // Phase 1: Create columns with horizontal splits
    let h_splits = cols - 1; // columns to add (1 already exists)
    for i in 0..h_splits {
        let cmd = format!("{}; exec bash", titled_cmd(&remaining_panes[i]));
        let output = tmux.split_window_pane(Some(target_session), true, Some(&cmd));
        if !output.status.success() {
            eprintln!("tms: h-split {} failed: {}",
                i, String::from_utf8_lossy(&output.stderr));
        }
    }

    // Even out the columns before vertical splits
    if h_splits > 0 {
        tmux.select_layout(target_session, "even-horizontal");
    }

    // Phase 2: Split columns vertically to add rows
    // After horizontal splits + even-horizontal, pane indices are sequential.
    // We need to split panes that need a second row.
    let panes_added = h_splits;
    let remaining_after_cols = &remaining_panes[panes_added..];

    for (i, pane) in remaining_after_cols.iter().enumerate() {
        // Target the pane by index within the window
        // After h_splits, panes are numbered 1..=(h_splits+1) (with base-pane-index 1)
        // We split pane (i+1) to add its bottom half
        let pane_target = format!("{}:.{}", target_session, i + 1);
        let cmd = format!("{}; exec bash", titled_cmd(pane));
        let output = tmux.split_window_pane(Some(&pane_target), false, Some(&cmd));
        if !output.status.success() {
            eprintln!("tms: v-split {} failed: {}",
                i, String::from_utf8_lossy(&output.stderr));
        }
    }
}
