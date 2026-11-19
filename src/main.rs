mod apps;
mod config;
mod model;

fn find(bundle_id: &str) -> Option<i32> {
    apps::list().into_iter().find(|a| a.bundle_id == bundle_id).map(|a| a.pid)
}

fn main() {
    println!("frontmost: {:?}", apps::frontmost_pid());
    for a in apps::list() {
        println!("{:>6} {:<8?} {:>12?} {}", a.pid, a.kind, a.memory_bytes, a.name);
    }
    std::process::Command::new("open").args(["-a", "Calculator"]).status().unwrap();
    std::thread::sleep(std::time::Duration::from_secs(2));
    let pid = find("com.apple.calculator").expect("Calculator should appear after a refresh");
    assert!(apps::quit(pid));
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert_eq!(find("com.apple.calculator"), None, "Calculator should be gone after quit");
    println!("smoke test passed");
}
