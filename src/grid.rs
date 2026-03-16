use crate::{
    tmux::Tmux,
    Result,
};

/// A pane to create in the grid — command to run + label for the border.
pub struct GridPane {
    pub command: String,
    pub label: String,
}

/// Build a tiled pane grid across one or more tmux windows.
///
/// If `session_name` is Some, creates a new session with that name.
/// If `session_name` is None, adds windows to the current session.
/// Each window gets up to `panes_per_window` panes in a "tiled" layout.
/// Each pane gets a border label set via its pane title.
pub fn build_pane_grid(
    tmux: &Tmux,
    session_name: Option<&str>,
    panes: Vec<GridPane>,
    panes_per_window: usize,
) -> Result<()> {
    if panes.is_empty() {
        return Ok(());
    }

    // Wrap command to set pane title before running
    let titled_cmd = |pane: &GridPane| -> String {
        // printf sets the pane title via escape sequence, then runs the command
        format!(
            "printf '\\033]2;{}\\033\\\\'; {}",
            pane.label.replace('\'', ""),
            pane.command
        )
    };

    // Determine the target session — either create new or use current
    let target_session: String = if let Some(name) = session_name {
        tmux.kill_session(name);

        let output = tmux.new_session(Some(name), None, Some(&titled_cmd(&panes[0])));
        if !output.status.success() {
            eprintln!(
                "tms: failed to create session '{}': {}",
                name,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        name.to_string()
    } else {
        let current = tmux.display_message("'#S'")
            .trim()
            .replace('\'', "");

        let output = tmux.new_window(None, None, Some(&current));
        if !output.status.success() {
            eprintln!(
                "tms: failed to create window: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        tmux.send_keys(&titled_cmd(&panes[0]), None);

        current
    };

    let mut current_window_pane_count: usize = 1;
    let mut window_start_indices: Vec<String> = Vec::new();

    // Track the current window for the final tiling pass
    let current_win_idx = tmux.display_message("'#{window_index}'")
        .trim()
        .replace('\'', "");
    if !current_win_idx.is_empty() {
        window_start_indices.push(current_win_idx);
    }

    for pane in panes.iter().skip(1) {
        let shell_cmd = format!("{}; exec bash", titled_cmd(pane));

        if current_window_pane_count >= panes_per_window {
            let output = tmux.new_window(None, None, Some(&target_session));
            if !output.status.success() {
                eprintln!(
                    "tms: failed to create window in '{}': {}",
                    target_session,
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            tmux.send_keys(&titled_cmd(pane), None);
            current_window_pane_count = 1;

            // Track new window index
            let win_idx = tmux.display_message("'#{window_index}'")
                .trim()
                .replace('\'', "");
            if !win_idx.is_empty() {
                window_start_indices.push(win_idx);
            }
        } else {
            let output = tmux.split_window_pane(Some(&target_session), Some(&shell_cmd));
            if !output.status.success() {
                eprintln!(
                    "tms: failed to split pane in '{}': {}",
                    target_session,
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            current_window_pane_count += 1;
            // No tiling here — wait until all panes in the window are created
        }
    }

    // Tile all windows once after all panes are created
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
