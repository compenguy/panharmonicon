#[cfg(windows)]
use windres::Build;

#[cfg(windows)]
fn main() {
    let rc = "assets/panharmonicon.rc";
    Build::new().compile(rc).expect(&format!("Failed to compiled windows resource file at {}", rc));
}

#[cfg(not(windows))]
fn main() {}