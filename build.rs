//! Embeds the app icon into the Windows executables, so the file has a face in Explorer and on
//! the taskbar. Does nothing anywhere else: on Linux the window icon set at run time is all there
//! is to set.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        // A failure here would mean no icon, which is not worth failing a build over.
        let _ = res.compile();
    }
}
