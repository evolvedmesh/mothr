//! Starting child processes from the desktop app.

use std::process::Command;

/// Keep a console program from opening a console window. Mothr is a GUI app on Windows (no
/// console of its own), so every agent, `git` or `--version` probe it starts would otherwise
/// flash up a window. The child still gets a hidden console, which its own children share.
/// Does nothing elsewhere.
pub fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}
