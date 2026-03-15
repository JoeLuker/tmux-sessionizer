use crate::{
    tmux::Tmux,
    Result,
};

/// Build a tiled pane grid across one or more tmux windows.
///
/// Creates a new session with the first command, then splits panes for the rest.
/// When a window reaches `panes_per_window`, a new window is created.
/// Each window gets a "tiled" layout applied after all panes are added.
pub fn build_pane_grid(
    tmux: &Tmux,
    session_name: &str,
    commands: Vec<String>,
    panes_per_window: usize,
) -> Result<()> {
    if commands.is_empty() {
        return Ok(());
    }

    // Kill old session if it exists
    tmux.kill_session(session_name);

    let mut current_window_pane_count: usize = 0;

    for (i, cmd) in commands.iter().enumerate() {
        let shell_cmd = format!("{}; exec bash", cmd);

        if i == 0 {
            // First pane: create session
            let output = tmux.new_session(Some(session_name), None, Some(cmd));
            if !output.status.success() {
                eprintln!(
                    "tms: failed to create session '{}': {}",
                    session_name,
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            current_window_pane_count = 1;
        } else if current_window_pane_count >= panes_per_window {
            // Window is full — create a new window (target session only, no window index)
            let output = tmux.new_window(None, None, Some(session_name));
            if !output.status.success() {
                eprintln!(
                    "tms: failed to create window in '{}': {}",
                    session_name,
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            // Send the command to the new window's pane
            tmux.send_keys(&cmd, None);
            current_window_pane_count = 1;
        } else {
            // Split in the session (tmux targets the most recent window by default)
            let target = session_name;
            let output = tmux.split_window_pane(Some(target), Some(&shell_cmd));
            if !output.status.success() {
                eprintln!(
                    "tms: failed to split pane in '{}': {}",
                    session_name,
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            current_window_pane_count += 1;

            // Re-tile after each split
            let tile_output = tmux.select_layout(target, "tiled");
            if !tile_output.status.success() {
                eprintln!(
                    "tms: failed to tile layout in '{}': {}",
                    session_name,
                    String::from_utf8_lossy(&tile_output.stderr)
                );
            }
        }
    }

    // Final tiling pass on all windows
    let windows_output = tmux.list_windows("#{window_index}", Some(session_name));
    for win_idx in windows_output.lines() {
        let win_idx = win_idx.trim();
        if !win_idx.is_empty() {
            let target = format!("{}:{}", session_name, win_idx);
            let output = tmux.select_layout(&target, "tiled");
            if !output.status.success() {
                eprintln!(
                    "tms: failed to tile window '{}': {}",
                    target,
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }

    // Select first window using ^ token (first window regardless of base-index)
    let first_window_target = format!("{}:^", session_name);
    tmux.select_window_by_token(&first_window_target);

    Ok(())
}
