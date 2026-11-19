use objc2_app_kit::{NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace};
use objc2_foundation::{NSDate, NSRunLoop};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Regular,
    MenuBar,
    Helper,
}

#[derive(Clone, Debug, PartialEq)]
pub struct App {
    pub name: String,
    pub bundle_id: String,
    pub pid: i32,
    pub kind: Kind,
    pub memory_bytes: Option<u64>,
    /// The .app bundle's path, used to look up its icon. Empty for processes without a bundle.
    pub bundle_path: String,
}

// A CLI has no Cocoa run loop, so NSWorkspace never sees launch/quit
// notifications unless we spin the loop ourselves.
fn pump_run_loop() {
    let until = NSDate::dateWithTimeIntervalSinceNow(0.01);
    NSRunLoop::currentRunLoop().runUntilDate(&until);
}

pub fn list() -> Vec<App> {
    pump_run_loop();
    NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .filter_map(|app| {
            let pid = app.processIdentifier();
            if pid <= 0 {
                return None;
            }
            let policy = app.activationPolicy();
            let kind = if policy == NSApplicationActivationPolicy::Regular {
                Kind::Regular
            } else if policy == NSApplicationActivationPolicy::Accessory {
                Kind::MenuBar
            } else {
                Kind::Helper
            };
            Some(App {
                name: app.localizedName().map(|s| s.to_string()).unwrap_or_default(),
                bundle_id: app.bundleIdentifier().map(|s| s.to_string()).unwrap_or_default(),
                pid,
                kind,
                memory_bytes: memory_bytes(pid),
                bundle_path: app.bundleURL().and_then(|u| u.path()).map(|p| p.to_string()).unwrap_or_default(),
            })
        })
        .collect()
}

// Same number Activity Monitor shows as "Memory".
fn memory_bytes(pid: i32) -> Option<u64> {
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V2, &mut info as *mut _ as *mut _)
    };
    (rc == 0).then_some(info.ri_phys_footprint)
}

pub fn frontmost_pid() -> Option<i32> {
    pump_run_loop();
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|app| app.processIdentifier())
}

pub fn quit(pid: i32) -> bool {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .is_some_and(|app| app.terminate())
}

pub fn force_quit(pid: i32) -> bool {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .is_some_and(|app| app.forceTerminate())
}

/// The icon macOS shows for `path` (an .app bundle), rasterised to a small PNG (64px on Retina).
pub fn icon_png(path: &str) -> Option<Vec<u8>> {
    use objc2::AllocAnyThread;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
    use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};
    let icon = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path));
    let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(32.0, 32.0));
    let cg = unsafe { icon.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
    let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
    let png = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }?;
    Some(png.to_vec())
}
