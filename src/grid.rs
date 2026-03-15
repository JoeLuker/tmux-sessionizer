use crate::{
    tmux::Tmux,
    Result,
};

/// Build a tiled pane grid across one or more tmux windows.
///
/// If `session_name` is Some, creates a new session with that name.
/// If `session_name` is None, adds windows to the current session.
/// Each window gets up to `panes_per_window` panes in a "tiled" layout.
pub fn build_pane_grid(
    tmux: &Tmux,
    session_name: Option<&str>,
    commands: Vec<String>,
    panes_per_window: usize,
) -> Result<()> {
    if commands.is_empty() {
        return Ok(());
    }

    // Determine the target session — either create new or use current
    let target_session: String = if let Some(name) = session_name {
        // New session mode: kill old, create fresh
        tmux.kill_session(name);

        let output = tmux.new_session(Some(name), None, Some(&commands[0]));
        if !output.status.success() {
            eprintln!(
                "tms: failed to create session '{}': {}",
                name,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        name.to_string()
    } else {
        // Current session mode: get current session name, create first window
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
        // Send the first command to the new window
        tmux.send_keys(&commands[0], None);

        current
    };

    let mut current_window_pane_count: usize = 1;

    for cmd in commands.iter().skip(1) {
        let shell_cmd = format!("{}; exec bash", cmd);

        if current_window_pane_count >= panes_per_window {
            // Window is full — create a new window
            let output = tmux.new_window(None, None, Some(&target_session));
            if !output.status.success() {
                eprintln!(
                    "tms: failed to create window in '{}': {}",
                    target_session,
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            tmux.send_keys(cmd, None);
            current_window_pane_count = 1;
        } else {
            // Split in the session (tmux targets the most recent window)
            let output = tmux.split_window_pane(Some(&target_session), Some(&shell_cmd));
            if !output.status.success() {
                eprintln!(
                    "tms: failed to split pane in '{}': {}",
                    target_session,
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            current_window_pane_count += 1;

            tmux.select_layout(&target_session, "tiled");
        }
    }

    // Final tiling pass on all windows
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
