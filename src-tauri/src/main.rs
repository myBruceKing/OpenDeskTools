#![cfg_attr(all(windows, not(feature = "console")), windows_subsystem = "windows")]

fn main() {
    open_desk_tools_lib::run();
}
