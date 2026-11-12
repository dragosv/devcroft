//! Experimental Linux entry point using devcroft's actual keeper implementation.
//! Listener creation and nono restriction precede every thread/session.
#[allow(dead_code, unused_imports)]
#[path = "../../../../src/keeper/mod.rs"]
mod keeper;

#[allow(dead_code)]
#[path = "../../../../src/ssh/sftp.rs"]
mod sftp;
#[allow(dead_code)]
#[path = "ssh_server.rs"]
mod ssh_server;

#[cfg(any(target_os = "linux", test))]
mod bootstrap;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
fn main() {
    if let Err(e) = linux::run() {
        eprintln!("guest keeper fatal: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("guest keeper requires Linux; host compilation is for unit tests only");
    std::process::exit(1);
}
