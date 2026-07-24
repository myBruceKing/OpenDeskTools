#[cfg(debug_assertions)]
fn main() {
    match open_desk_tools_lib::write_debug_screenshot_probe_report() {
        Ok(path) => println!("{}", path.display()),
        Err(error) => {
            eprintln!("screenshot probe failed: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(debug_assertions))]
fn main() {
    eprintln!("the screenshot probe is available only in debug builds");
    std::process::exit(1);
}
