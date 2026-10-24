//! Embeds the icon Explorer, the taskbar and the tray show, and the version details Explorer's
//! Properties and Task Manager read.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    winresource::WindowsResource::new()
        // Resource ID 1, which Explorer shows for the exe and `load_icon` loads.
        .set_icon("assets/icon.ico")
        // Task Manager names the process by its description, so it is the app's name, not the
        // package's one-line description.
        .set("FileDescription", "Zilum")
        .set("ProductName", "Zilum")
        .compile()
        .expect("the icon and version details compile into a resource");
}
