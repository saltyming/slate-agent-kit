//! The `slate-setup` executable: runs the installer with the real process environment.
//!
//! Owns nothing but the hand-off from the operating system to [`slate_setup::run`].

use slate_setup::env::Env;

fn main() {
    let env = Env::from_process();
    let code = slate_setup::run(std::env::args_os(), &env, None);
    std::process::exit(code);
}
